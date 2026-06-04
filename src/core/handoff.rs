use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::schemas::{AuditResponse, DevResponse, Handoff, HandoffStatus};

pub fn create_dev_to_auditor_handoff(
    _run_id: &str,
    step_id: &str,
    dev_response: &DevResponse,
    patch_hash: &str,
) -> Handoff {
    let mut decisions = vec![format!("Patch hash: {patch_hash}")];
    if !dev_response.tests_suggested.is_empty() {
        decisions.push(format!(
            "Testes sugeridos: {}",
            dev_response.tests_suggested.join("; ")
        ));
    }
    Handoff {
        agent: "dev".to_string(),
        target_agent: "auditor".to_string(),
        step_id: step_id.to_string(),
        status: HandoffStatus::WaitingAudit,
        summary: dev_response.summary.clone(),
        decisions,
        files_touched: dev_response.files_touched.clone(),
        open_questions: Vec::new(),
        risks: dev_response.risks.clone(),
        next_action: "Revisar o patch, validar escopo e auditar riscos de segurança.".to_string(),
    }
}

pub fn create_auditor_to_dev_handoff(
    _run_id: &str,
    step_id: &str,
    audit_response: &AuditResponse,
) -> Handoff {
    Handoff {
        agent: "auditor".to_string(),
        target_agent: "dev".to_string(),
        step_id: step_id.to_string(),
        status: HandoffStatus::Rejected,
        summary: audit_response
            .blocked_reason
            .clone()
            .unwrap_or_else(|| "Patch rejected by auditor".to_string()),
        decisions: audit_response.required_changes.clone(),
        files_touched: Vec::new(),
        open_questions: audit_response.required_changes.clone(),
        risks: audit_response.problems.clone(),
        next_action: "Revise the implementation and submit a new patch for review.".to_string(),
    }
}

pub fn save_handoff(
    orchestrator_dir: &Path,
    run_id: &str,
    from: &str,
    to: &str,
    handoff: &Handoff,
) -> Result<()> {
    handoff.validate()?;
    let path = orchestrator_dir
        .join("handoffs")
        .join(format!("{run_id}-{from}-to-{to}.json"));
    let content = serde_json::to_string_pretty(handoff)?;
    fs::write(&path, content)
        .with_context(|| format!("failed to write handoff file {}", path.display()))
}

pub fn load_last_handoff(orchestrator_dir: &Path) -> Result<Option<Handoff>> {
    Ok(load_last_handoff_with_meta(orchestrator_dir)?.map(|entry| entry.0))
}

pub fn load_last_handoff_with_meta(
    orchestrator_dir: &Path,
) -> Result<Option<(Handoff, PathBuf, SystemTime)>> {
    let handoffs_dir = orchestrator_dir.join("handoffs");
    if !handoffs_dir.exists() {
        return Ok(None);
    }

    let mut latest: Option<(Handoff, PathBuf, SystemTime)> = None;

    for entry in fs::read_dir(&handoffs_dir)
        .with_context(|| format!("failed to read directory {}", handoffs_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }

        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata for {}", path.display()))?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read handoff file {}", path.display()))?;
        let handoff: Handoff = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse handoff file {}", path.display()))?;

        let should_replace = latest
            .as_ref()
            .map(|(_, _, current)| modified > *current)
            .unwrap_or(true);

        if should_replace {
            latest = Some((handoff, path, modified));
        }
    }

    Ok(latest)
}
