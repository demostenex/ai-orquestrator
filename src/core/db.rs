use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

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
    status       TEXT NOT NULL,      -- "waiting_audit" | "approved" | "rejected" | "ready_for_dev" | "waiting_human"
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

#[derive(Clone)]
pub struct Db {
    pool: Pool<SqliteConnectionManager>,
    project_id: String,
    run_id: String,
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
        
        // C1: Habilitar modo WAL via with_init para propagar a todas as conexões do pool
        let manager = SqliteConnectionManager::file(&db_path)
            .with_init(|c| c.pragma_update(None, "journal_mode", "WAL"));
        
        let pool = Pool::new(manager)?;

        let conn = pool.get()?;
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
            pool,
            project_id,
            run_id: run_id.to_string(),
        })
    }

    /// Registra um evento na fila ordenada do run.
    pub async fn log_event(
        &self,
        event_type: EventType,
        from_agent: Option<&str>,
        to_agent: Option<&str>,
        content_summary: Option<&str>,
        content_hash: Option<&str>,
        enriched_by_human: bool,
        human_notes: Option<&str>,
    ) -> Result<()> {
        let pool = self.pool.clone();
        let pid = self.project_id.clone();
        let rid = self.run_id.clone();
        let etype = event_type.as_str().to_string();
        let from = from_agent.map(|s| s.to_string());
        let to = to_agent.map(|s| s.to_string());
        let summary = content_summary.map(|s| s.to_string());
        let hash = content_hash.map(|s| s.to_string());
        let notes = human_notes.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let sequence: i64 = conn.query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM events WHERE run_id = ?1",
                params![rid],
                |r| r.get(0)
            )?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO events
                 (project_id, run_id, sequence, event_type, from_agent, to_agent,
                  content_summary, content_hash, enriched_by_human, human_notes, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![pid, rid, sequence, etype, from, to, summary, hash, enriched_by_human as i64, notes, now],
            )?;
            Ok(())
        }).await?
    }

    /// Registra um handoff.
    pub async fn log_handoff(
        &self,
        from_agent: &str,
        to_agent: &str,
        status: &str,
        summary: &str,
        decisions: Vec<String>,
        risks: Vec<String>,
        open_questions: Vec<String>,
        files_touched: Vec<String>,
        next_action: &str,
    ) -> Result<()> {
        let pool = self.pool.clone();
        let pid = self.project_id.clone();
        let rid = self.run_id.clone();
        let from = from_agent.to_string();
        let to = to_agent.to_string();
        let st = status.to_string();
        let sum = summary.to_string();
        let dec = serde_json::to_string(&decisions)?;
        let rsk = serde_json::to_string(&risks)?;
        let qst = serde_json::to_string(&open_questions)?;
        let fls = serde_json::to_string(&files_touched)?;
        let nxt = next_action.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO handoffs
                 (project_id, run_id, from_agent, to_agent, status, summary,
                  decisions, risks, open_questions, files_touched, next_action, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![pid, rid, from, to, st, sum, dec, rsk, qst, fls, nxt, now],
            )?;
            Ok(())
        }).await?
    }

    /// Atualiza o status e campos finais do run corrente.
    pub async fn update_run_status(
        &self,
        status: &str,
        patch_hash: Option<&str>,
        audit_approved: Option<bool>,
        audit_score: Option<i64>,
        memory_hash: Option<&str>,
    ) -> Result<()> {
        let pool = self.pool.clone();
        let rid = self.run_id.clone();
        let st = status.to_string();
        let phash = patch_hash.map(|s| s.to_string());
        let approved = audit_approved.map(|v| v as i64);
        let score = audit_score;
        let mhash = memory_hash.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE runs SET status = ?1, patch_hash = COALESCE(?2, patch_hash),
                 audit_approved = COALESCE(?3, audit_approved),
                 audit_score = COALESCE(?4, audit_score),
                 memory_hash = COALESCE(?5, memory_hash),
                 updated_at = ?6
                 WHERE id = ?7",
                params![st, phash, approved, score, mhash, now, rid],
            )?;
            Ok(())
        }).await?
    }

    // ── Novos Métodos CRUD (Passo 1.3/1.4) ─────────────────────────────────────

    pub async fn create_plan(&self, plan_id: &str, title: &str) -> Result<()> {
        let pool = self.pool.clone();
        let pid = self.project_id.clone();
        let rid = self.run_id.clone();
        let id = plan_id.to_string();
        let t = title.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO plans (id, project_id, run_id, title, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'draft', ?5, ?5)",
                params![id, pid, rid, t, now],
            )?;
            Ok(())
        }).await?
    }

    pub async fn add_plan_turn(&self, plan_id: &str, agent: &str, prompt: &str, content: &str) -> Result<()> {
        let pool = self.pool.clone();
        let plid = plan_id.to_string();
        let ag = agent.to_string();
        let p = prompt.to_string();
        let c = content.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let sequence: i64 = conn.query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM plan_turns WHERE plan_id = ?1",
                params![plid],
                |r| r.get(0)
            )?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO plan_turns (plan_id, sequence, agent, prompt, content, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![plid, sequence, ag, p, c, now],
            )?;
            Ok(())
        }).await?
    }

    pub async fn add_task(&self, task_id: &str, plan_id: &str, description: &str, assigned_to: Option<&str>, sequence: i64) -> Result<()> {
        let pool = self.pool.clone();
        let tid = task_id.to_string();
        let plid = plan_id.to_string();
        let desc = description.to_string();
        let ass = assigned_to.map(|s| s.to_string());
        
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO tasks (id, plan_id, description, status, assigned_to, sequence, write_locked, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, 0, ?6, ?6)",
                params![tid, plid, desc, ass, sequence, now],
            )?;
            Ok(())
        }).await?
    }

    /// Ativa o bloqueio de escrita em todas as tarefas de um plano.
    /// Deve ser chamado **somente** após confirmação explícita do usuário.
    pub async fn lock_plan_tasks(&self, plan_id: &str) -> Result<()> {
        let pool = self.pool.clone();
        let plid = plan_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            conn.execute(
                "UPDATE tasks SET write_locked = 1 WHERE plan_id = ?1",
                params![plid],
            )?;
            Ok(())
        }).await?
    }

    pub async fn update_task_status(&self, agent_name: &str, task_id: &str, status: &str) -> Result<()> {
        let pool = self.pool.clone();
        let agent = agent_name.to_string();
        let tid = task_id.to_string();
        let st = status.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            
            // CA3: Validação de write_locked - Permitir somente IA Dev se bloqueado
            let is_locked: bool = conn.query_row(
                "SELECT write_locked FROM tasks WHERE id = ?1",
                params![tid],
                |r| r.get::<_, i64>(0).map(|v| v != 0)
            )?;

            if is_locked && agent != "dev" {
                anyhow::bail!("Tarefa {} bloqueada para escrita (agente: {}).", tid, agent);
            }

            let now = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![st, now, tid],
            )?;
            Ok(())
        }).await?
    }

    // CA-MD1: Métodos de leitura para exportação
    pub async fn get_plan_title(&self, plan_id: &str) -> Result<String> {
        let pool = self.pool.clone();
        let plid = plan_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            conn.query_row(
                "SELECT title FROM plans WHERE id = ?1",
                params![plid],
                |r| r.get(0)
            ).map_err(Into::into)
        }).await?
    }

    pub async fn get_plan_tasks(&self, plan_id: &str) -> Result<Vec<crate::schemas::Task>> {
        let pool = self.pool.clone();
        let plid = plan_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let mut stmt = conn.prepare(
                "SELECT id, description, status, assigned_to FROM tasks WHERE plan_id = ?1 ORDER BY sequence ASC"
            )?;
            let rows = stmt.query_map(params![plid], |row| {
                Ok(crate::schemas::Task {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    status: row.get(2)?,
                    assigned_to: row.get(3)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }).await?
    }

    pub async fn get_plan_turns(&self, plan_id: &str) -> Result<Vec<crate::schemas::PlanTurn>> {
        let pool = self.pool.clone();
        let plid = plan_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let mut stmt = conn.prepare(
                "SELECT sequence, agent, prompt, content, timestamp FROM plan_turns WHERE plan_id = ?1 ORDER BY sequence ASC"
            )?;
            let rows = stmt.query_map(params![plid], |row| {
                Ok(crate::schemas::PlanTurn {
                    sequence: row.get(0)?,
                    agent: row.get(1)?,
                    prompt: row.get(2)?,
                    content: row.get(3)?,
                    timestamp: row.get(4)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }).await?
    }

    // CA-MD2: Registrar versão com hash
    pub async fn add_plan_version(&self, plan_id: &str, content_hash: &str, notes: Option<&str>) -> Result<()> {
        let pool = self.pool.clone();
        let plid = plan_id.to_string();
        let hash = content_hash.to_string();
        let nts = notes.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO plan_versions (plan_id, content_hash, human_notes, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![plid, hash, nts, now],
            )?;
            Ok(())
        }).await?
    }

    // CA-MD3: Buscar último hash para detecção de conflito
    pub async fn get_latest_plan_hash(&self, plan_id: &str) -> Result<Option<String>> {
        let pool = self.pool.clone();
        let plid = plan_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let mut stmt = conn.prepare(
                "SELECT content_hash FROM plan_versions WHERE plan_id = ?1 ORDER BY created_at DESC LIMIT 1"
            )?;
            let res = stmt.query_row(params![plid], |r| r.get(0)).optional()?;
            Ok(res)
        }).await?
    }

    // ── Queries de leitura ────────────────────────────────────────────────────

    pub async fn list_events(&self) -> Result<Vec<EventRow>> {
        let pool = self.pool.clone();
        let rid = self.run_id.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            let mut stmt = conn.prepare(
                "SELECT id, run_id, sequence, event_type, from_agent, to_agent,
                        content_summary, enriched_by_human, human_notes, timestamp
                 FROM events WHERE run_id = ?1 ORDER BY sequence ASC",
            )?;
            let rows = stmt.query_map(params![rid], |row| {
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
        }).await?
    }

    /// Lista os últimos N runs do projeto, com seus eventos resumidos.
    pub async fn list_recent_runs(orchestrator_dir: PathBuf, workspace_path: PathBuf, limit: usize) -> Result<Vec<RunRow>> {
        tokio::task::spawn_blocking(move || {
            let db_path = orchestrator_dir.join("history.db");
            if !db_path.exists() {
                return Ok(vec![]);
            }
            let conn = rusqlite::Connection::open(&db_path)?;
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
        }).await?
    }

    /// Lista todos os eventos de um run específico (para auditoria).
    pub async fn list_events_for_run(
        orchestrator_dir: PathBuf,
        run_id: String,
    ) -> Result<Vec<EventRow>> {
        tokio::task::spawn_blocking(move || {
            let db_path = orchestrator_dir.join("history.db");
            let conn = rusqlite::Connection::open(&db_path)?;
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
        }).await?
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }
}
