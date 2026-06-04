use std::io::{self};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use chrono::Utc;
use colored::Colorize;
use serde_json::{json, Value};

use crate::commands::{
    load_cycle, print_step, print_success, read_to_string, save_cycle, write_json,
};
use crate::core::config::Config;
use crate::core::db::{Db, EventType};
use crate::core::git;
use crate::core::handoff::load_last_handoff;
use crate::core::patch::{compute_patch_hash, parse_diff};
use crate::core::security::scan_diff;
use crate::schemas::CycleStatus;

pub async fn execute(patch: Option<PathBuf>, dry_run: bool) -> Result<()> {
    let config = Config::load()?;
    let mut cycle = load_cycle(&config.orchestrator_dir)?;

    if cycle.status != CycleStatus::Approved {
        return Err(anyhow!(
            "current cycle is not approved. Run `ai-orchestrator run` or `ai-orchestrator audit` first."
        ));
    }

    let patch_path = match patch {
        Some(path) if path.is_absolute() => path,
        Some(path) => config.workspace_dir.join(path),
        None => {
            let relative = cycle
                .patch_file
                .clone()
                .ok_or_else(|| anyhow!("current cycle does not reference a patch file"))?;
            config.orchestrator_dir.join(relative)
        }
    };

    let audit_file = cycle
        .audit_file
        .clone()
        .ok_or_else(|| anyhow!("current cycle does not reference an audit response"))?;
    let audit_path = config.orchestrator_dir.join(audit_file);
    let audit_raw = read_to_string(&audit_path)?;
    let audit_json: Value = serde_json::from_str(&audit_raw)?;
    let approved = audit_json
        .pointer("/parsed/approved")
        .or_else(|| audit_json.get("approved"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !approved {
        return Err(anyhow!("latest audit did not approve this patch"));
    }

    if dry_run {
        // Rigorous check even in dry-run
        let diff = read_to_string(&patch_path)?;
        let patch_hash = compute_patch_hash(&diff);
        if cycle.patch_hash.as_deref() != Some(patch_hash.as_str()) {
            return Err(anyhow!(
                "patch hash mismatch: patch file content changed since approval"
            ));
        }
        git::apply_check(&config.workspace_dir, &patch_path)?;
        print_success("Dry-run concluído. Nenhuma alteração aplicada.");
        return Ok(());
    }

    apply_patch_rigorous(&config, &mut cycle, &patch_path, &audit_json, false).await
}

/// Executa a aplicação rigorosa de um patch, com todas as verificações de segurança e integridade.
/// Centraliza a lógica para uso no CLI, TUI e run_dev_cycle.
pub async fn apply_patch_rigorous(
    config: &Config,
    cycle: &mut crate::schemas::CycleState,
    patch_path: &std::path::Path,
    audit_json: &Value,
    force_apply: bool,
) -> Result<()> {
    let diff = read_to_string(patch_path)?;
    let patch_hash = compute_patch_hash(&diff);

    // 1. Validar Hash
    if cycle.patch_hash.as_deref() != Some(patch_hash.as_str()) {
        return Err(anyhow!(
            "patch hash mismatch: patch file content changed since approval"
        ));
    }

    // 2. Segurança
    if !scan_diff(&diff).is_empty() {
        return Err(anyhow!(
            "security violations detected in patch. Refusing to apply."
        ));
    }

    // 3. Integridade do Git
    let current_head = git::get_head_commit(&config.workspace_dir)?;
    if current_head != cycle.base_commit {
        return Err(anyhow!(
            "HEAD moved since patch generation. Expected {}, found {}.",
            cycle.base_commit,
            current_head
        ));
    }

    if !git::is_clean_tree(&config.workspace_dir)? {
        return Err(anyhow!("working tree is dirty. Refusing to apply patch."));
    }

    // 4. Check antes de aplicar
    git::apply_check(&config.workspace_dir, patch_path)?;

    // 5. Preview e Confirmação (apenas se não for force_apply)
    let parsed_diff = parse_diff(&diff)?;
    let score = audit_json
        .pointer("/parsed/score")
        .or_else(|| audit_json.get("score"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    if !force_apply {
        let last_handoff = load_last_handoff(&config.orchestrator_dir)?;
        let risks = last_handoff
            .as_ref()
            .map(|h| h.risks.clone())
            .unwrap_or_default();

        println!("{}", "══════════════════════════════════════".bold());
        println!("{}", "ATENÇÃO: Revisão final antes de aplicar".bold());
        println!("{}", "══════════════════════════════════════".bold());
        println!("Patch   : {}", patch_path.display());
        println!("Arquivos: {}", parsed_diff.files_modified.join(", "));
        println!("Score   : {}", score);
        println!(
            "Riscos  : {}",
            if risks.is_empty() {
                "Nenhum crítico".to_string()
            } else {
                risks.join(", ")
            }
        );
        println!("{}", "──────── preview (50 linhas) ────────".dimmed());
        for line in diff.lines().take(50) {
            println!("{line}");
        }
        println!("{}", "══════════════════════════════════════".bold());

        print_step("Digite APPLY para aplicar o patch:");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if input.trim() != "APPLY" {
            return Err(anyhow!(
                "application aborted: confirmation text did not match APPLY"
            ));
        }
    }

    // 6. Aplicação Real
    git::apply_patch(&config.workspace_dir, patch_path)?;
    print_success("Patch aplicado com sucesso.");

    // 7. Commit Automático
    let last_handoff = load_last_handoff(&config.orchestrator_dir)?;
    let handoff_summary = last_handoff
        .as_ref()
        .map(|h| h.summary.clone())
        .unwrap_or_else(|| cycle.step_id.clone());

    let commit_msg = format!(
        "[ai-orchestrator] step {}: {} (score: {}, arquivos: {})",
        cycle.step_id,
        handoff_summary,
        score,
        parsed_diff.files_modified.join(", "),
    );

    git::add_all(&config.workspace_dir)?;
    git::commit(&config.workspace_dir, &commit_msg)?;
    print_success(&format!("Commit: {}", commit_msg.green()));

    // 8. Atualizar Estado
    cycle.status = CycleStatus::Applied;
    cycle.updated_at = Utc::now();
    save_cycle(&config.orchestrator_dir, cycle)?;

    // 9. Registrar no Banco e Logs
    if let Ok(db) = Db::open(
        &config.orchestrator_dir,
        &config.workspace_dir,
        &cycle.run_id,
        &cycle.step_id,
        "apply",
        &cycle.base_commit,
        "",
        "",
    ) {
        let _ = db
            .log_event(
                EventType::ApplyConfirmed,
                Some("human"),
                Some("git"),
                Some("Humano confirmou aplicação do patch"),
                Some(&patch_hash),
                true,
                Some(if force_apply {
                    "Confirmado via TUI/Gate"
                } else {
                    "APPLY confirmado manualmente"
                }),
            )
            .await;
        let _ = db
            .log_event(
                EventType::PatchApplied,
                Some("git"),
                None,
                Some(&format!(
                    "Patch aplicado e commitado: {} arquivo(s) — {}",
                    parsed_diff.files_modified.len(),
                    commit_msg
                )),
                Some(&patch_hash),
                false,
                None,
            )
            .await;
        let _ = db
            .update_run_status(
                "applied",
                Some(&patch_hash),
                Some(true),
                Some(score as i64),
                None,
            )
            .await;
    }

    let log_path = config
        .orchestrator_dir
        .join("logs")
        .join(format!("{}-apply.json", cycle.run_id));
    write_json(
        &log_path,
        &json!({
            "timestamp": Utc::now().to_rfc3339(),
            "patch_file": patch_path.display().to_string(),
            "patch_hash": patch_hash,
            "audit_score": score,
            "files_applied": parsed_diff.files_modified,
            "commit_message": commit_msg,
            "confirmed_by": if force_apply { "tui" } else { "human" },
            "result": "success"
        }),
    )?;

    Ok(())
}
