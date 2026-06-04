use std::fs;

use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use uuid::Uuid;

use crate::commands::{print_step, print_success, save_cycle, write_json, write_string};
use crate::core::compute_sha256;
use crate::core::config::{detect_workspace_dir, StoredConfig};
use crate::core::git;
use crate::core::memory::LastSyncState;
use crate::schemas::{CycleState, CycleStatus};

const PLAN_TEMPLATE: &str = r#"# Plano de execução

## Objetivo
- Descreva o que a IA Dev deve implementar.

## Escopo
- Liste arquivos e módulos permitidos.

## Critérios de aceite
- Defina testes e comportamentos esperados.
"#;

const MEMORY_TEMPLATE: &str = r#"# IA-Memory

## Decisões arquiteturais
- Registre aqui decisões, restrições e aprendizados relevantes.
"#;

pub async fn execute(force: bool, dry_run: bool) -> Result<()> {
    let workspace_dir = detect_workspace_dir()?;
    let orchestrator_dir = workspace_dir.join(".ai-orchestrator");

    if orchestrator_dir.exists() && !force {
        anyhow::bail!(
            ".ai-orchestrator already exists at {}. Use --force to reinitialize.",
            orchestrator_dir.display()
        );
    }

    if dry_run {
        print_step(&format!("Dry-run em {}", workspace_dir.display()));
        println!("Would create {}", orchestrator_dir.display());
        println!("Would write config.json, plan.md, current-cycle.json, memory/context.md and supporting directories.");
        return Ok(());
    }

    if orchestrator_dir.exists() {
        fs::remove_dir_all(&orchestrator_dir)?;
    }

    print_step("Criando estrutura .ai-orchestrator/...");
    fs::create_dir_all(orchestrator_dir.join("memory"))?;
    fs::create_dir_all(orchestrator_dir.join("mailbox"))?;
    fs::create_dir_all(orchestrator_dir.join("handoffs"))?;
    fs::create_dir_all(orchestrator_dir.join("patches"))?;
    fs::create_dir_all(orchestrator_dir.join("logs"))?;
    fs::create_dir_all(orchestrator_dir.join("memory-sync"))?;

    let stored_config = StoredConfig::default();
    write_json(&orchestrator_dir.join("config.json"), &stored_config)?;
    write_string(&orchestrator_dir.join("plan.md"), PLAN_TEMPLATE)?;
    write_string(
        &orchestrator_dir.join("memory").join("context.md"),
        MEMORY_TEMPLATE,
    )?;
    write_json(
        &orchestrator_dir.join("memory-sync").join("last-sync.json"),
        &LastSyncState {
            last_sync: None,
            memory_hash: None,
        },
    )?;

    // Garantir que .ai-orchestrator/ está no .gitignore para não sujar o working tree
    let gitignore_path = workspace_dir.join(".gitignore");
    let entry = ".ai-orchestrator/\n";
    let already_ignored = if gitignore_path.exists() {
        let content = fs::read_to_string(&gitignore_path).unwrap_or_default();
        content.contains(".ai-orchestrator")
    } else {
        false
    };
    if !already_ignored {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore_path)?;
        file.write_all(entry.as_bytes())?;
        print_step("Adicionado .ai-orchestrator/ ao .gitignore");
    }

    let plan_hash = compute_sha256(PLAN_TEMPLATE);
    let memory_hash = compute_sha256(MEMORY_TEMPLATE);
    let base_commit = git::ensure_repository(&workspace_dir)?;
    let cycle = CycleState {
        run_id: Uuid::new_v4().to_string(),
        step_id: stored_config.step_id.clone(),
        status: CycleStatus::Initialized,
        base_commit,
        plan_hash,
        memory_hash,
        patch_file: None,
        patch_hash: None,
        audit_file: None,
        audit_approved: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    save_cycle(&orchestrator_dir, &cycle)?;

    print_success("AI Orchestrator inicializado com sucesso.");
    println!(
        "{}",
        "Edite .ai-orchestrator/plan.md antes de executar `ai-orchestrator run`.".green()
    );
    println!(
        "{}",
        format!("Timestamp: {}", Utc::now().to_rfc3339()).dimmed()
    );
    Ok(())
}
