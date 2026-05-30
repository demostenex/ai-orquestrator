use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::compute_sha256;
use crate::schemas::{Handoff, HandoffStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastSyncState {
    pub last_sync: Option<DateTime<Utc>>,
    pub memory_hash: Option<String>,
}

pub fn sync_handoff_summary(orchestrator_dir: &Path, run_id: &str, handoff: &Handoff) -> Result<String> {
    let memory_path = orchestrator_dir.join("memory").join("context.md");
    let sync_path = orchestrator_dir.join("memory-sync").join("last-sync.json");
    let timestamp = Utc::now();

    let agent_label = match handoff.agent.as_str() {
        "dev"      => "IA Dev",
        "auditor"  => "IA Auditora",
        "architect"=> "IA Arquiteta",
        other      => other,
    };
    let status_label = match handoff.status {
        HandoffStatus::WaitingAudit => "Aguardando auditoria",
        HandoffStatus::Approved     => "Aprovado",
        HandoffStatus::Rejected     => "Reprovado — requer correções",
        HandoffStatus::ReadyForDev  => "Pronto para Dev",
        HandoffStatus::WaitingHuman => "Aguardando humano",
    };

    let files_line = if handoff.files_touched.is_empty() {
        "  (nenhum arquivo)".to_string()
    } else {
        handoff.files_touched.iter().map(|f| format!("  - {f}")).collect::<Vec<_>>().join("\n")
    };
    let decisions_line = if handoff.decisions.is_empty() {
        "  (nenhuma)".to_string()
    } else {
        handoff.decisions.iter().map(|d| format!("  - {d}")).collect::<Vec<_>>().join("\n")
    };
    let risks_line = if handoff.risks.is_empty() {
        "  (nenhum)".to_string()
    } else {
        handoff.risks.iter().map(|r| format!("  - {r}")).collect::<Vec<_>>().join("\n")
    };
    let questions_line = if handoff.open_questions.is_empty() {
        "  (nenhuma)".to_string()
    } else {
        handoff.open_questions.iter().map(|q| format!("  - {q}")).collect::<Vec<_>>().join("\n")
    };

    let section = format!(
        "\n## [{agent_label}] Run {run_id} — {ts}\n\
        **Status:** {status_label}  \n\
        **Resumo:** {summary}\n\
        **Próxima ação:** {next}\n\n\
        **Arquivos tocados:**\n{files_line}\n\n\
        **Decisões tomadas:**\n{decisions_line}\n\n\
        **Riscos identificados:**\n{risks_line}\n\n\
        **Dúvidas em aberto:**\n{questions_line}\n",
        ts       = timestamp.to_rfc3339(),
        summary  = if handoff.summary.is_empty() { "(sem resumo)" } else { &handoff.summary },
        next     = if handoff.next_action.is_empty() { "(sem próxima ação definida)" } else { &handoff.next_action },
    );

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&memory_path)
        .with_context(|| format!("failed to open memory context {}", memory_path.display()))?;
    file.write_all(section.as_bytes())
        .with_context(|| format!("failed to update memory context {}", memory_path.display()))?;

    let content = fs::read_to_string(&memory_path)
        .with_context(|| format!("failed to read memory context {}", memory_path.display()))?;
    let memory_hash = compute_sha256(&content);
    let sync_state = LastSyncState {
        last_sync: Some(timestamp),
        memory_hash: Some(memory_hash.clone()),
    };

    fs::write(&sync_path, serde_json::to_string_pretty(&sync_state)?)
        .with_context(|| format!("failed to write memory sync file {}", sync_path.display()))?;

    Ok(memory_hash)
}

/// Escreve uma página no ai-memory via CLI e retorna o path da página criada (o "recibo").
/// Retorna None se o CLI não estiver disponível ou falhar.
pub fn write_to_ai_memory(wiki_path: &str, title: &str, body: &str) -> Option<String> {
    use std::io::Write as _;

    let mut child = std::process::Command::new("ai-memory")
        .args(["write-page", "--path", wiki_path, "--title", title, "--body", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes());
    }

    let output = child.wait_with_output().ok()?;
    if !output.status.success() { return None; }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    // Tenta extrair "path" do JSON retornado pelo CLI
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stdout) {
        if let Some(path) = val.get("path").and_then(|p| p.as_str()) {
            return Some(path.to_string());
        }
    }
    let trimmed = stdout.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

pub fn load_last_sync(orchestrator_dir: &Path) -> Result<LastSyncState> {
    let sync_path = orchestrator_dir.join("memory-sync").join("last-sync.json");
    if !sync_path.exists() {
        return Ok(LastSyncState {
            last_sync: None,
            memory_hash: None,
        });
    }

    let raw = fs::read_to_string(&sync_path)
        .with_context(|| format!("failed to read memory sync file {}", sync_path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse memory sync file {}", sync_path.display()))
}
