use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};

use crate::core::compute_sha256;

// ── Schema ────────────────────────────────────────────────────────────────────

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id           TEXT PRIMARY KEY,   -- sha256 do workspace_path
    workspace_path TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,      -- último segmento do path (ex: "lcconnect")
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runs (
    id           TEXT PRIMARY KEY,   -- UUID
    project_id   TEXT NOT NULL REFERENCES projects(id),
    step_id      TEXT NOT NULL,
    status       TEXT NOT NULL,
    mode         TEXT NOT NULL,      -- "auto" | "manual"
    base_commit  TEXT NOT NULL,
    plan_hash    TEXT NOT NULL,
    memory_hash  TEXT NOT NULL,
    patch_hash   TEXT,
    audit_approved INTEGER,          -- 1=aprovado, 0=reprovado, NULL=pendente
    audit_score  INTEGER,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id   TEXT NOT NULL REFERENCES projects(id),
    run_id       TEXT NOT NULL REFERENCES runs(id),
    sequence     INTEGER NOT NULL,   -- ordem dentro do run
    event_type   TEXT NOT NULL,      -- ver EventType
    from_agent   TEXT,
    to_agent     TEXT,
    content_summary TEXT,
    content_hash TEXT,
    enriched_by_human INTEGER DEFAULT 0,
    human_notes  TEXT,
    timestamp    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS handoffs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id   TEXT NOT NULL REFERENCES projects(id),
    run_id       TEXT NOT NULL REFERENCES runs(id),
    from_agent   TEXT NOT NULL,
    to_agent     TEXT NOT NULL,
    status       TEXT NOT NULL,
    summary      TEXT,
    decisions    TEXT,               -- JSON array serializado
    risks        TEXT,               -- JSON array serializado
    open_questions TEXT,             -- JSON array serializado
    files_touched  TEXT,             -- JSON array serializado
    next_action  TEXT,
    timestamp    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS security_violations (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id   TEXT NOT NULL REFERENCES projects(id),
    run_id       TEXT NOT NULL,
    pattern      TEXT NOT NULL,
    line_number  INTEGER,
    timestamp    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plans (
    id           TEXT PRIMARY KEY,   -- UUID do plano
    project_id   TEXT NOT NULL REFERENCES projects(id),
    run_id       TEXT,               -- NULL se o plano ainda não foi associado a um run
    title        TEXT NOT NULL,      -- Título humano para o plano
    status       TEXT NOT NULL,      -- "draft" | "approved" | "completed"
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    FOREIGN KEY(run_id) REFERENCES runs(id)
);

CREATE TABLE IF NOT EXISTS plan_versions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id      TEXT NOT NULL REFERENCES plans(id),
    content_hash TEXT NOT NULL,      -- hash do conteúdo na época
    human_notes  TEXT,               -- modificações do usuário (gate)
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plan_turns (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id      TEXT NOT NULL REFERENCES plans(id),
    sequence     INTEGER NOT NULL,   -- ordem dos turnos no planejamento
    agent        TEXT NOT NULL,      -- "architect" | "dev"
    prompt       TEXT NOT NULL,      -- O que foi enviado à IA
    content      TEXT NOT NULL,      -- Resposta da IA (JSON ou texto)
    timestamp    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    id           TEXT PRIMARY KEY,   -- UUID da tarefa
    plan_id      TEXT NOT NULL REFERENCES plans(id),
    description  TEXT NOT NULL,
    status       TEXT NOT NULL,      -- "pending" | "in_progress" | "completed" | "blocked"
    assigned_to  TEXT,               -- nome do agente (ex: "dev")
    sequence     INTEGER NOT NULL,   -- ordem na lista
    write_locked INTEGER DEFAULT 0,  -- 1=apenas IA Dev pode escrever após plano finalizado
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_runs_project   ON runs(project_id);
CREATE INDEX IF NOT EXISTS idx_events_run     ON events(run_id);
CREATE INDEX IF NOT EXISTS idx_events_project ON events(project_id);
CREATE INDEX IF NOT EXISTS idx_handoffs_run   ON handoffs(run_id);
CREATE INDEX IF NOT EXISTS idx_plans_run      ON plans(run_id);
CREATE INDEX IF NOT EXISTS idx_tasks_plan     ON tasks(plan_id);
CREATE INDEX IF NOT EXISTS idx_turns_plan     ON plan_turns(plan_id);
"#;

// ── Tipos de evento ───────────────────────────────────────────────────────────

pub enum EventType {
    PromptSent,
    ResponseReceived,
    GatePassed,
    GateEnriched,
    HandoffCreated,
    SecurityCheckPassed,
    SecurityBlocked,
    GitCheckPassed,
    GitCheckFailed,
    PatchSaved,
    AuditApproved,
    AuditRejected,
    ApplyConfirmed,
    PatchApplied,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PromptSent => "prompt_sent",
            Self::ResponseReceived => "response_received",
            Self::GatePassed => "gate_passed",
            Self::GateEnriched => "gate_enriched",
            Self::HandoffCreated => "handoff_created",
            Self::SecurityCheckPassed => "security_check_passed",
            Self::SecurityBlocked => "security_blocked",
            Self::GitCheckPassed => "git_check_passed",
            Self::GitCheckFailed => "git_check_failed",
            Self::PatchSaved => "patch_saved",
            Self::AuditApproved => "audit_approved",
            Self::AuditRejected => "audit_rejected",
            Self::ApplyConfirmed => "apply_confirmed",
            Self::PatchApplied => "patch_applied",
        }
    }
}

// ── Structs de leitura ────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct EventRow {
    pub id: i64,
    pub run_id: String,
    pub sequence: i64,
    pub event_type: String,
    pub from_agent: Option<String>,
    pub to_agent: Option<String>,
    pub content_summary: Option<String>,
    pub enriched_by_human: bool,
    pub human_notes: Option<String>,
    pub timestamp: String,
}

#[derive(Debug)]
pub struct RunRow {
    pub id: String,
    pub project_name: String,
    pub step_id: String,
    pub status: String,
    pub mode: String,
    pub base_commit: String,
    pub audit_approved: Option<bool>,
    pub audit_score: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

// ── Handle do banco ───────────────────────────────────────────────────────────

pub struct Db {
    conn: Connection,
    project_id: String,
    run_id: String,
    sequence: i64,
}

impl Db {
    /// Abre (ou cria) o banco em `.ai-orchestrator/history.db`.
    /// Registra o projeto pelo workspace_path e o run corrente.
    pub fn open(
        orchestrator_dir: &Path,
        workspace_path: &Path,
        run_id: &str,
        step_id: &str,
        mode: &str,
        base_commit: &str,
        plan_hash: &str,
        memory_hash: &str,
    ) -> Result<Self> {
        let db_path = orchestrator_dir.join("history.db");
        let conn = Connection::open(&db_path)?;

        // Habilitar modo WAL antes de executar o schema (CA2)
        conn.pragma_update(None, "journal_mode", "WAL")?;

        conn.execute_batch(SCHEMA)?;

        let workspace_str = workspace_path.to_string_lossy().to_string();
        let project_id = compute_sha256(&workspace_str);
        let project_name = workspace_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| workspace_str.clone());
        let now = Utc::now().to_rfc3339();

        // Upsert do projeto
        conn.execute(
            "INSERT OR IGNORE INTO projects (id, workspace_path, name, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![project_id, workspace_str, project_name, now],
        )?;

        // Insert do run
        conn.execute(
            "INSERT INTO runs
             (id, project_id, step_id, status, mode, base_commit, plan_hash, memory_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'initialized', ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                run_id,
                project_id,
                step_id,
                mode,
                base_commit,
                plan_hash,
                memory_hash,
                now,
            ],
        )?;

        Ok(Self {
            conn,
            project_id,
            run_id: run_id.to_string(),
            sequence: 0,
        })
    }

    /// Registra um evento na fila ordenada do run.
    pub fn log_event(
        &mut self,
        event_type: EventType,
        from_agent: Option<&str>,
        to_agent: Option<&str>,
        content_summary: Option<&str>,
        content_hash: Option<&str>,
        enriched_by_human: bool,
        human_notes: Option<&str>,
    ) -> Result<()> {
        self.sequence += 1;
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO events
             (project_id, run_id, sequence, event_type, from_agent, to_agent,
              content_summary, content_hash, enriched_by_human, human_notes, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                self.project_id,
                self.run_id,
                self.sequence,
                event_type.as_str(),
                from_agent,
                to_agent,
                content_summary,
                content_hash,
                enriched_by_human as i64,
                human_notes,
                now,
            ],
        )?;
        Ok(())
    }

    /// Registra um handoff.
    pub fn log_handoff(
        &self,
        from_agent: &str,
        to_agent: &str,
        status: &str,
        summary: &str,
        decisions: &[String],
        risks: &[String],
        open_questions: &[String],
        files_touched: &[String],
        next_action: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO handoffs
             (project_id, run_id, from_agent, to_agent, status, summary,
              decisions, risks, open_questions, files_touched, next_action, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                self.project_id,
                self.run_id,
                from_agent,
                to_agent,
                status,
                summary,
                serde_json::to_string(decisions).unwrap_or_default(),
                serde_json::to_string(risks).unwrap_or_default(),
                serde_json::to_string(open_questions).unwrap_or_default(),
                serde_json::to_string(files_touched).unwrap_or_default(),
                next_action,
                now,
            ],
        )?;
        Ok(())
    }

    /// Atualiza o status e campos finais do run corrente.
    pub fn update_run_status(
        &self,
        status: &str,
        patch_hash: Option<&str>,
        audit_approved: Option<bool>,
        audit_score: Option<i64>,
        memory_hash: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE runs SET status = ?1, patch_hash = COALESCE(?2, patch_hash),
             audit_approved = COALESCE(?3, audit_approved),
             audit_score = COALESCE(?4, audit_score),
             memory_hash = COALESCE(?5, memory_hash),
             updated_at = ?6
             WHERE id = ?7",
            params![
                status,
                patch_hash,
                audit_approved.map(|v| v as i64),
                audit_score,
                memory_hash,
                now,
                self.run_id,
            ],
        )?;
        Ok(())
    }

    // ── Queries de leitura ────────────────────────────────────────────────────

    /// Lista todos os eventos do run atual em ordem de sequência.
    pub fn list_events(&self) -> Result<Vec<EventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, sequence, event_type, from_agent, to_agent,
                    content_summary, enriched_by_human, human_notes, timestamp
             FROM events WHERE run_id = ?1 ORDER BY sequence ASC",
        )?;
        let rows = stmt.query_map(params![self.run_id], |row| {
            Ok(EventRow {
                id: row.get(0)?,
                run_id: row.get(1)?,
                sequence: row.get(2)?,
                event_type: row.get(3)?,
                from_agent: row.get(4)?,
                to_agent: row.get(5)?,
                content_summary: row.get(6)?,
                enriched_by_human: row.get::<_, i64>(7)? != 0,
                human_notes: row.get(8)?,
                timestamp: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lista os últimos N runs do projeto, com seus eventos resumidos.
    pub fn list_recent_runs(orchestrator_dir: &Path, workspace_path: &Path, limit: usize) -> Result<Vec<RunRow>> {
        let db_path = orchestrator_dir.join("history.db");
        if !db_path.exists() {
            return Ok(vec![]);
        }
        let conn = Connection::open(&db_path)?;
        let workspace_str = workspace_path.to_string_lossy().to_string();
        let project_id = compute_sha256(&workspace_str);
        let project_name = workspace_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| workspace_str.clone());

        let mut stmt = conn.prepare(
            "SELECT id, step_id, status, mode, base_commit,
                    audit_approved, audit_score, created_at, updated_at
             FROM runs WHERE project_id = ?1
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![project_id, limit as i64], |row| {
            Ok(RunRow {
                id: row.get(0)?,
                project_name: project_name.clone(),
                step_id: row.get(1)?,
                status: row.get(2)?,
                mode: row.get(3)?,
                base_commit: row.get(4)?,
                audit_approved: row.get::<_, Option<i64>>(5)?.map(|v| v != 0),
                audit_score: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Lista todos os eventos de um run específico (para auditoria).
    pub fn list_events_for_run(
        orchestrator_dir: &Path,
        run_id: &str,
    ) -> Result<Vec<EventRow>> {
        let db_path = orchestrator_dir.join("history.db");
        let conn = Connection::open(&db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, sequence, event_type, from_agent, to_agent,
                    content_summary, enriched_by_human, human_notes, timestamp
             FROM events WHERE run_id = ?1 ORDER BY sequence ASC",
        )?;
        let rows = stmt.query_map(params![run_id], |row| {
            Ok(EventRow {
                id: row.get(0)?,
                run_id: row.get(1)?,
                sequence: row.get(2)?,
                event_type: row.get(3)?,
                from_agent: row.get(4)?,
                to_agent: row.get(5)?,
                content_summary: row.get(6)?,
                enriched_by_human: row.get::<_, i64>(7)? != 0,
                human_notes: row.get(8)?,
                timestamp: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }
}
