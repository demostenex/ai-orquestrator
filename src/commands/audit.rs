use std::path::PathBuf;

use anyhow::{anyhow, Result};
use chrono::Utc;
use colored::Colorize;
use serde_json::json;
use uuid::Uuid;

use crate::agents::auditor;
use crate::commands::{
    latest_patch_file, load_cycle, print_step, print_success, read_to_string, save_cycle,
    write_json,
};
use crate::core::compute_sha256;
use crate::core::config::Config;
use crate::core::git;
use crate::core::handoff::{create_auditor_to_dev_handoff, load_last_handoff, save_handoff};
use crate::core::memory::sync_handoff_summary;
use crate::core::patch::{compute_patch_hash, parse_diff};
use crate::providers::create_provider;
use crate::schemas::{CycleState, CycleStatus};

pub async fn execute(patch: Option<PathBuf>, dry_run: bool) -> Result<()> {
    let config = Config::load()?;
    let patch_path = match patch {
        Some(path) if path.is_absolute() => path,
        Some(path) => config.workspace_dir.join(path),
        None => latest_patch_file(&config.orchestrator_dir)?
            .ok_or_else(|| anyhow!("no patch file found in {}", config.orchestrator_dir.join("patches").display()))?,
    };

    if !git::is_clean_tree(&config.workspace_dir)? {
        return Err(anyhow!(
            "working tree is dirty. Commit or stash your changes before running `ai-orchestrator audit`."
        ));
    }

    print_step(&format!("Carregando patch: {}", patch_path.display()));
    let diff = read_to_string(&patch_path)?;
    let parsed_diff = parse_diff(&diff)?;
    let patch_hash = compute_patch_hash(&diff);
    git::apply_check(&config.workspace_dir, &patch_path)?;
    print_success("git apply --check passou.");

    let plan = read_to_string(&config.orchestrator_dir.join("plan.md"))?;
    let memory = read_to_string(&config.orchestrator_dir.join("memory").join("context.md"))?;
    let last_handoff = load_last_handoff(&config.orchestrator_dir)?;

    if dry_run {
        print_success("Dry-run concluído.");
        println!("Patch      : {}", patch_path.display());
        println!("Patch hash : {patch_hash}");
        return Ok(());
    }

    let provider = create_provider(&config)?;
    let run_id = Uuid::new_v4().to_string();
    let step_id = load_cycle(&config.orchestrator_dir)
        .map(|cycle| cycle.step_id)
        .unwrap_or_else(|_| config.step_id.clone());

    let audit_request_path = config
        .orchestrator_dir
        .join("mailbox")
        .join(format!("{run_id}-audit-request.json"));
    let prompt = auditor::build_user_prompt(
        &step_id,
        &plan,
        &memory,
        last_handoff.as_ref(),
        &diff,
        &parsed_diff.files_modified,
        "git apply --check: success",
    );
    write_json(
        &audit_request_path,
        &json!({
            "run_id": run_id,
            "step_id": step_id,
            "system": auditor::system_prompt(),
            "prompt": prompt,
            "patch_hash": patch_hash,
        }),
    )?;

    print_step("Solicitando auditoria...");
    let audit_call = auditor::execute(
        provider.as_ref(),
        &step_id,
        &plan,
        &memory,
        last_handoff.as_ref(),
        &diff,
        &parsed_diff.files_modified,
        "git apply --check: success",
    )
    .await?;

    let audit_response_path = config
        .orchestrator_dir
        .join("mailbox")
        .join(format!("{run_id}-audit-response.json"));
    write_json(
        &audit_response_path,
        &json!({
            "run_id": run_id,
            "provider": audit_call.provider_response.provider,
            "model": audit_call.provider_response.model,
            "finish_reason": audit_call.provider_response.finish_reason,
            "raw_text": audit_call.provider_response.text,
            "parsed": audit_call.parsed,
        }),
    )?;

    let now = Utc::now();
    let mut cycle = load_cycle(&config.orchestrator_dir).unwrap_or(CycleState {
        run_id: run_id.clone(),
        step_id: step_id.clone(),
        status: CycleStatus::Initialized,
        base_commit: git::get_head_commit(&config.workspace_dir).unwrap_or_else(|_| "unknown".to_string()),
        plan_hash: compute_sha256(&plan),
        memory_hash: compute_sha256(&memory),
        patch_file: None,
        patch_hash: None,
        audit_file: None,
        audit_approved: None,
        created_at: now,
        updated_at: now,
    });
    cycle.run_id = run_id.clone();
    cycle.step_id = step_id.clone();
    cycle.patch_file = Some(
        patch_path
            .strip_prefix(&config.orchestrator_dir)
            .unwrap_or(&patch_path)
            .display()
            .to_string(),
    );
    cycle.patch_hash = Some(patch_hash.clone());
    cycle.audit_file = Some(format!("mailbox/{run_id}-audit-response.json"));
    cycle.audit_approved = Some(audit_call.parsed.approved);
    cycle.status = if audit_call.parsed.approved {
        CycleStatus::Approved
    } else {
        CycleStatus::Rejected
    };
    cycle.updated_at = Utc::now();
    save_cycle(&config.orchestrator_dir, &cycle)?;

    let mut handoff_log = Vec::new();
    if !audit_call.parsed.approved {
        let handoff = create_auditor_to_dev_handoff(&run_id, &step_id, &audit_call.parsed);
        save_handoff(&config.orchestrator_dir, &run_id, "auditor", "dev", &handoff)?;
        sync_handoff_summary(&config.orchestrator_dir, &run_id, &handoff)?;
        handoff_log.push(handoff);
    }

    let log_path = config
        .orchestrator_dir
        .join("logs")
        .join(format!("{run_id}-audit.json"));
    write_json(
        &log_path,
        &json!({
            "timestamp": Utc::now().to_rfc3339(),
            "run_id": run_id,
            "patch_file": patch_path.display().to_string(),
            "patch_hash": patch_hash,
            "audit_response": audit_call.parsed,
            "handoffs": handoff_log,
        }),
    )?;

    println!("\n{}", "══════════════════════════════════════".bold());
    println!("{}", "RESULTADO DA AUDITORIA".bold());
    println!("{}", "══════════════════════════════════════".bold());
    println!(
        "Status  : {}",
        if audit_call.parsed.approved {
            "APROVADO".green().bold().to_string()
        } else {
            "REPROVADO".red().bold().to_string()
        }
    );
    println!("Score   : {}", audit_call.parsed.score);
    println!("Arquivos: {}", parsed_diff.files_modified.join(", "));
    println!(
        "Problemas: {}",
        if audit_call.parsed.problems.is_empty() {
            "Nenhum".to_string()
        } else {
            audit_call.parsed.problems.join("; ")
        }
    );
    println!("{}", "══════════════════════════════════════".bold());
    println!("{}", "Execute `ai-orchestrator apply` para aplicar após revisão.".yellow());

    Ok(())
}
