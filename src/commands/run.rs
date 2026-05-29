use anyhow::{anyhow, Result};
use chrono::Utc;
use colored::Colorize;
use serde_json::json;
use uuid::Uuid;

use crate::agents::{auditor, dev};
use crate::commands::{
    interactive_gate, print_manual_instructions, print_step, print_success,
    print_warning, read_to_string, save_cycle, wait_for_file, write_json, write_string,
};
use crate::core::cli_runner::{is_cli_available, run_cli, select_cli};
use crate::core::compute_sha256;
use crate::core::config::{Config, StoredConfig};
use crate::core::db::{Db, EventType};
use crate::core::git;
use crate::core::handoff::{create_auditor_to_dev_handoff, create_dev_to_auditor_handoff, load_last_handoff, save_handoff};
use crate::core::memory::sync_handoff_summary;
use crate::core::patch::{compute_patch_hash, parse_diff};
use crate::core::security::scan_diff;
use crate::core::strip_json_fences;
use crate::providers::create_provider;
use crate::schemas::{AuditResponse, CycleState, CycleStatus, DevResponse};

/// Timeout de espera em modo manual: 30 minutos
const MANUAL_TIMEOUT_SECS: u64 = 1800;

fn print_agent_prompt(role: &str, cli: &str, prompt: &str) {
    let sep = "══════════════════════════════════════";
    println!("\n{}", sep.bold());
    println!("{}", format!("📤 PROMPT → {role} ({cli})").bold().cyan());
    println!("{}", sep.bold());
    for line in prompt.lines() {
        println!("  {line}");
    }
    println!("{}", sep.bold());
    println!("{}", format!("⏳ {role} ({cli}) TRABALHANDO...").bold().yellow());
    println!("{}", sep.bold());
}

fn print_agent_done(role: &str, cli: &str) {
    let sep = "══════════════════════════════════════";
    println!("{}", sep.bold());
    println!("{}", format!("✔ {role} ({cli}) — CONCLUÍDO").bold().green());
    println!("{}", sep.bold());
}

/// Modo de execução do agente
enum AgentMode {
    /// Chama a API diretamente via provider configurado
    Api,
    /// Pipar prompt para um CLI externo
    Cli(String),
    /// Aguardar arquivo salvo manualmente pelo usuário
    Manual,
}

/// Resolve o modo de execução: CLI salvo no config → CLI em PATH → seleção interativa
fn resolve_agent_mode(saved_cli: Option<&str>, role: &str) -> Result<AgentMode> {
    // Já tem CLI salvo no config?
    if let Some(cli) = saved_cli {
        if is_cli_available(cli) {
            println!("  {} CLI: {}", role, cli.green());
            return Ok(AgentMode::Cli(cli.to_string()));
        } else {
            println!("{}", format!("⚠ CLI '{cli}' salvo no config não encontrado. Selecionando novamente...").yellow());
        }
    }
    // Menu interativo
    match select_cli(role)? {
        Some(cli) => Ok(AgentMode::Cli(cli)),
        None => Ok(AgentMode::Manual),
    }
}

pub async fn execute(step: Option<String>, dry_run: bool, manual: bool, new_plan: bool) -> Result<()> {
    let mut config = Config::load()?;
    let step_id = step.unwrap_or_else(|| config.step_id.clone());

    if !git::is_clean_tree(&config.workspace_dir)? {
        return Err(anyhow!(
            "working tree is dirty. Commit or stash your changes before running `ai-orchestrator run`."
        ));
    }

    let plan_path = config.orchestrator_dir.join("plan.md");
    let memory_path = config.orchestrator_dir.join("memory").join("context.md");

    // ── Plano: digitar no terminal se não existe ou --new-plan ────────────────
    let plan = if new_plan || !plan_path.exists() {
        if new_plan {
            print_warning("--new-plan está depreciado (decisão O1 do Passo 4).");
            println!("  Use {} no futuro para criar planos.", "ai-orchestrator plan new".cyan());
            println!();
        }

        println!("\n{}", "══════════════════════════════════════".bold());
        println!("{}", "📋  NOVO PLANO".bold().cyan());
        println!("{}", "══════════════════════════════════════".bold());
        println!("  Digite o plano abaixo. Uma linha em branco finaliza.\n");
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if line.trim().is_empty() { break; }
            lines.push(line);
        }
        if lines.is_empty() {
            return Err(anyhow!("plano vazio. Informe ao menos uma linha."));
        }
        let content = lines.concat();
        std::fs::write(&plan_path, &content)?;
        print_success("Plano salvo em plan.md.");
        content
    } else {
        print_step("Lendo plan.md...");
        read_to_string(&plan_path)?
    };

    let plan_hash = compute_sha256(&plan);

    print_step("Carregando contexto IA-Memory...");
    let memory = read_to_string(&memory_path)?;
    let memory_hash = compute_sha256(&memory);
    let last_handoff = load_last_handoff(&config.orchestrator_dir)?;

    if dry_run {
        print_success("Dry-run concluído.");
        println!("Plan hash   : {plan_hash}");
        println!("Memory hash : {memory_hash}");
        println!("Step        : {step_id}");
        return Ok(());
    }

    // ── Selecionar CLIs ───────────────────────────────────────────────────────
    println!("\n{}", "══════════════════════════════════════".bold());
    println!("{}", "CONFIGURAÇÃO DOS AGENTES".bold());
    println!("{}", "══════════════════════════════════════".bold());

    let dev_mode = if manual {
        AgentMode::Manual
    } else {
        resolve_agent_mode(config.dev_cli.as_deref(), "Dev")?
    };

    let audit_mode = if manual {
        AgentMode::Manual
    } else {
        resolve_agent_mode(config.audit_cli.as_deref(), "Auditora")?
    };

    // Salvar escolhas no config para próximas execuções
    let dev_cli_name = match &dev_mode { AgentMode::Cli(c) => Some(c.clone()), _ => None };
    let audit_cli_name = match &audit_mode { AgentMode::Cli(c) => Some(c.clone()), _ => None };

    if dev_cli_name != config.dev_cli || audit_cli_name != config.audit_cli {
        config.dev_cli = dev_cli_name.clone();
        config.audit_cli = audit_cli_name.clone();
        let stored = StoredConfig {
            provider: config.provider.clone(),
            model: config.model.clone(),
            step_id: config.step_id.clone(),
            dev_cli: dev_cli_name,
            audit_cli: audit_cli_name,
        };
        write_json(&config.orchestrator_dir.join("config.json"), &stored)?;
        print_step("Configuração salva.");
    }

    println!("  Dev      : {}", match &dev_mode { AgentMode::Cli(c) => c.green().to_string(), AgentMode::Api => "API".cyan().to_string(), AgentMode::Manual => "manual".yellow().to_string() });
    println!("  Auditora : {}", match &audit_mode { AgentMode::Cli(c) => c.green().to_string(), AgentMode::Api => "API".cyan().to_string(), AgentMode::Manual => "manual".yellow().to_string() });
    println!("{}", "══════════════════════════════════════".bold());

    let base_commit = git::get_head_commit(&config.workspace_dir)?;
    let run_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let mut cycle = CycleState {
        run_id: run_id.clone(),
        step_id: step_id.clone(),
        status: CycleStatus::Initialized,
        base_commit: base_commit.clone(),
        plan_hash: plan_hash.clone(),
        memory_hash: memory_hash.clone(),
        patch_file: None,
        patch_hash: None,
        audit_file: None,
        audit_approved: None,
        created_at: now,
        updated_at: now,
    };
    save_cycle(&config.orchestrator_dir, &cycle)?;

    // ── Abrir banco de histórico ──────────────────────────────────────────────
    let db = Db::open(
        &config.orchestrator_dir,
        &config.workspace_dir,
        &run_id,
        &step_id,
        if manual { "manual" } else { "auto" },
        &base_commit,
        &plan_hash,
        &memory_hash,
    )?;

    // ── Paths dos arquivos de mailbox ─────────────────────────────────────────
    let dev_request_path  = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-dev-request.json"));
    let dev_prompt_txt    = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-dev-prompt.txt"));
    let dev_response_path = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-dev-response.json"));

    // ── Prompt Dev ────────────────────────────────────────────────────────────
    print_step("Lendo estado do workspace...");
    let workspace_snapshot = dev::build_workspace_snapshot(&config.workspace_dir);
    let dev_user_prompt = dev::build_user_prompt(&step_id, &plan, &memory, last_handoff.as_ref(), &workspace_snapshot);
    let dev_system = dev::system_prompt();

    write_json(
        &dev_request_path,
        &json!({
            "run_id": run_id,
            "step_id": step_id,
            "system": dev_system,
            "prompt": dev_user_prompt,
            "plan_hash": plan_hash,
            "memory_hash": memory_hash,
        }),
    )?;

    // Portão 1: revisar prompt antes de enviar para Dev (apenas no modo manual)
    let final_dev_user_prompt = {
        let raw_prompt = format!("=== SISTEMA ===\n{dev_system}\n\n=== USUÁRIO ===\n{dev_user_prompt}");
        match &dev_mode {
            AgentMode::Manual => {
                let enriched = interactive_gate("Prompt para IA Dev", &raw_prompt, "Orquestrador → IA Dev").await?;
                let had_notes = enriched.contains("[NOTAS DO HUMANO]");
                let notes = if had_notes { enriched.splitn(2, "[NOTAS DO HUMANO]").nth(1).map(str::trim).map(str::to_string) } else { None };
                db.log_event(if had_notes { EventType::GateEnriched } else { EventType::GatePassed }, Some("orchestrator"), Some("dev"), Some("Prompt revisado pelo humano antes de enviar para Dev"), None, had_notes, notes.as_deref()).await?;
                enriched
            }
            _ => {
                db.log_event(EventType::PromptSent, Some("orchestrator"), Some("dev"), Some("Prompt enviado para IA Dev"), None, false, None).await?;
                raw_prompt
            }
        }
    };

    write_string(&dev_prompt_txt, &final_dev_user_prompt)?;

    // ── Chamar IA Dev ─────────────────────────────────────────────────────────
    let dev_response: DevResponse = match &dev_mode {
        AgentMode::Manual => {
            print_manual_instructions("IA Dev", &dev_prompt_txt, &dev_response_path);
            wait_for_file(&dev_response_path, MANUAL_TIMEOUT_SECS).await?;
            let raw = read_to_string(&dev_response_path)?;
            let parsed: DevResponse = serde_json::from_str(strip_json_fences(&raw))
                .map_err(|e| anyhow!("resposta da IA Dev inválida: {e}"))?;
            parsed.validate()?;
            print_success("Resposta da IA Dev recebida e validada.");
            parsed
        }
        AgentMode::Cli(cli) => {
            print_agent_prompt("IA DEV", cli, &final_dev_user_prompt);
            let raw = run_cli(cli, &final_dev_user_prompt)?;
            print_agent_done("IA DEV", cli);
            write_string(&dev_response_path, &raw)?;
            let parsed: DevResponse = serde_json::from_str(strip_json_fences(&raw))
                .map_err(|e| anyhow!("resposta do CLI '{cli}' inválida: {e}"))?;
            parsed.validate()?;
            print_success("Resposta da IA Dev recebida e validada.");
            parsed
        }
        AgentMode::Api => {
            print_step("Solicitando implementação para IA Dev via API...");
            let provider = create_provider(&config)?;
            let call = dev::execute(provider.as_ref(), &step_id, &plan, &memory, last_handoff.as_ref(), &workspace_snapshot).await?;
            write_json(&dev_response_path, &json!({
                "run_id": run_id,
                "provider": call.provider_response.provider,
                "model": call.provider_response.model,
                "raw_text": call.provider_response.text,
                "parsed": call.parsed,
            }))?;
            print_success("Resposta recebida e validada.");
            call.parsed
        }
    };

    db.log_event(
        EventType::ResponseReceived,
        Some("dev"),
        Some("orchestrator"),
        Some(&format!("Dev respondeu: {} arquivo(s) modificado(s), {} risco(s)",
            dev_response.files_touched.len(), dev_response.risks.len())),
        None,
        false,
        None,
    ).await?;

    // ── Segurança + Patch ─────────────────────────────────────────────────────
    let violations = scan_diff(&dev_response.diff);
    if !violations.is_empty() {
        for v in &violations {
            db.log_event(EventType::SecurityBlocked, Some("security"), None, Some(&format!("Violação: {:?}", v)), None, false, None).await?;
        }
        db.update_run_status("security_blocked", None, None, None, None).await?;
        return Err(anyhow!("security violations detected in patch: {:?}", violations));
    }
    db.log_event(EventType::SecurityCheckPassed, Some("security"), None, Some("Nenhuma violação de segurança detectada"), None, false, None).await?;

    let parsed_diff = parse_diff(&dev_response.diff)?;
    let patch_hash = compute_patch_hash(&parsed_diff.raw);
    let patch_path = config.orchestrator_dir.join("patches").join(format!("{run_id}.diff"));
    write_string(&patch_path, &parsed_diff.raw)?;
    db.log_event(EventType::PatchSaved, Some("orchestrator"), None, Some(&format!("Patch salvo: {run_id}.diff (hash: {patch_hash})")), Some(&patch_hash), false, None).await?;

    git::apply_check(&config.workspace_dir, &patch_path)?;
    print_success("git apply --check passou.");
    db.log_event(EventType::GitCheckPassed, Some("git"), None, Some("git apply --check passou"), None, false, None).await?;

    let mut normalized_dev = dev_response.clone();
    normalized_dev.files_touched = parsed_diff.files_modified.clone();
    cycle.status = CycleStatus::DevDone;
    cycle.patch_file = Some(format!("patches/{run_id}.diff"));
    cycle.patch_hash = Some(patch_hash.clone());
    cycle.updated_at = Utc::now();
    save_cycle(&config.orchestrator_dir, &cycle)?;

    // Portão 2: revisar diff antes de passar para Auditora (sempre — independente do modo)
    {
        let diff_preview = format!(
            "Arquivos modificados: {}\nRiscos: {}\n\n--- DIFF ---\n{}",
            normalized_dev.files_touched.join(", "),
            if normalized_dev.risks.is_empty() { "nenhum".to_string() } else { normalized_dev.risks.join(", ") },
            parsed_diff.raw,
        );
        let gate2 = interactive_gate("Resposta da IA Dev → Auditora", &diff_preview, "IA Dev → IA Auditora").await?;
        let had_notes = gate2.contains("[NOTAS DO HUMANO]");
        let notes = if had_notes { gate2.splitn(2, "[NOTAS DO HUMANO]").nth(1).map(str::trim).map(str::to_string) } else { None };
        db.log_event(
            if had_notes { EventType::GateEnriched } else { EventType::GatePassed },
            Some("dev"), Some("auditor"),
            Some("Diff revisado pelo humano antes de enviar à Auditora"),
            Some(&patch_hash), had_notes, notes.as_deref(),
        ).await?;
    }

    print_step("Criando handoff dev → auditor...");
    let dev_handoff = create_dev_to_auditor_handoff(&run_id, &step_id, &normalized_dev, &patch_hash);
    save_handoff(&config.orchestrator_dir, &run_id, "dev", "auditor", &dev_handoff)?;
    db.log_event(EventType::HandoffCreated, Some("dev"), Some("auditor"), Some(&dev_handoff.summary), None, false, None).await?;
    db.log_handoff(
        "dev", "auditor", "waiting_audit", &dev_handoff.summary,
        dev_handoff.decisions.clone(), dev_handoff.risks.clone(),
        dev_handoff.open_questions.clone(), dev_handoff.files_touched.clone(),
        &dev_handoff.next_action
    ).await?;

    print_step("Sincronizando com IA-Memory...");
    let synced_memory_hash = sync_handoff_summary(&config.orchestrator_dir, &run_id, &dev_handoff)?;
    // Recarrega a memória atualizada para que a Auditora veja o contexto mais recente
    let memory = read_to_string(&memory_path).unwrap_or(memory);
    cycle.memory_hash = synced_memory_hash;
    cycle.status = CycleStatus::AuditRequested;
    cycle.updated_at = Utc::now();
    save_cycle(&config.orchestrator_dir, &cycle)?;

    db.update_run_status("dev_done", Some(&patch_hash), None, None, Some(&cycle.memory_hash)).await?;

    // ── Paths auditoria ───────────────────────────────────────────────────────
    let audit_request_path = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-audit-request.json"));
    let audit_prompt_txt  = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-audit-prompt.txt"));
    let audit_response_path = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-audit-response.json"));

    let audit_user_prompt = auditor::build_user_prompt(
        &step_id,
        &plan,
        &memory,
        Some(&dev_handoff),
        &parsed_diff.raw,
        &parsed_diff.files_modified,
        "git apply --check: success",
    );
    let audit_system = auditor::system_prompt();

    // Portão 3: revisar prompt antes de enviar para Auditora
    let final_audit_prompt = {
        let raw = format!("=== SISTEMA ===\n{audit_system}\n\n=== USUÁRIO ===\n{audit_user_prompt}");
        match &audit_mode {
            AgentMode::Manual => {
                let enriched = interactive_gate("Prompt para IA Auditora", &raw, "Orquestrador → IA Auditora").await?;
                let had_notes = enriched.contains("[NOTAS DO HUMANO]");
                let notes = if had_notes { enriched.splitn(2, "[NOTAS DO HUMANO]").nth(1).map(str::trim).map(str::to_string) } else { None };
                db.log_event(if had_notes { EventType::GateEnriched } else { EventType::GatePassed }, Some("orchestrator"), Some("auditor"), Some("Prompt para Auditora revisado pelo humano"), None, had_notes, notes.as_deref()).await?;
                enriched
            }
            _ => {
                db.log_event(EventType::PromptSent, Some("orchestrator"), Some("auditor"), Some("Prompt enviado para IA Auditora"), None, false, None).await?;
                raw
            }
        }
    };

    write_json(
        &audit_request_path,
        &json!({
            "run_id": run_id,
            "step_id": step_id,
            "system": audit_system,
            "prompt": audit_user_prompt,
            "patch_hash": patch_hash,
            "files_modified": parsed_diff.files_modified,
        }),
    )?;

    write_string(&audit_prompt_txt, &final_audit_prompt)?;

    // ── Chamar IA Auditora ────────────────────────────────────────────────────
    let audit_response: AuditResponse = match &audit_mode {
        AgentMode::Manual => {
            print_manual_instructions("IA Auditora", &audit_prompt_txt, &audit_response_path);
            wait_for_file(&audit_response_path, MANUAL_TIMEOUT_SECS).await?;
            let raw = read_to_string(&audit_response_path)?;
            let parsed: AuditResponse = serde_json::from_str(strip_json_fences(&raw))
                .map_err(|e| anyhow!("resposta da IA Auditora inválida: {e}"))?;
            parsed.validate()?;
            print_success("Resposta da IA Auditora recebida e validada.");
            parsed
        }
        AgentMode::Cli(cli) => {
            print_agent_prompt("IA AUDITORA", cli, &final_audit_prompt);
            let raw = run_cli(cli, &final_audit_prompt)?;
            print_agent_done("IA AUDITORA", cli);
            write_string(&audit_response_path, &raw)?;
            let parsed: AuditResponse = serde_json::from_str(strip_json_fences(&raw))
                .map_err(|e| anyhow!("resposta do CLI '{cli}' inválida: {e}"))?;
            parsed.validate()?;
            print_success("Resposta da IA Auditora recebida e validada.");
            parsed
        }
        AgentMode::Api => {
            print_step("Solicitando auditoria via API...");
            let provider = create_provider(&config)?;
            let call = auditor::execute(provider.as_ref(), &step_id, &plan, &memory, Some(&dev_handoff), &parsed_diff.raw, &normalized_dev.files_touched, "git apply --check: success").await?;
            write_json(&audit_response_path, &json!({
                "run_id": run_id,
                "provider": call.provider_response.provider,
                "model": call.provider_response.model,
                "raw_text": call.provider_response.text,
                "parsed": call.parsed,
            }))?;
            call.parsed
        }
    };

    db.log_event(
        EventType::ResponseReceived,
        Some("auditor"),
        Some("orchestrator"),
        Some(&format!("Auditora respondeu: aprovado={}, score={}", audit_response.approved, audit_response.score)),
        None, false, None,
    ).await?;

    // Portão 4: sempre mostra resultado e pede confirmação do humano
    {
        let audit_summary = format!(
            "Status   : {}\nScore    : {}\nProblemas: {}\nBloqueio : {}",
            if audit_response.approved { "APROVADO" } else { "REPROVADO" },
            audit_response.score,
            if audit_response.problems.is_empty() { "nenhum".to_string() } else { audit_response.problems.join("; ") },
            audit_response.blocked_reason.as_deref().unwrap_or("—"),
        );
        interactive_gate("Resultado da Auditoria", &audit_summary, "IA Auditora → Humano").await?;
    }

    // ── Resultado ─────────────────────────────────────────────────────────────
    let mut log_handoffs = vec![json!(dev_handoff)];
    let final_status = if audit_response.approved {
        CycleStatus::Approved
    } else {
        CycleStatus::Rejected
    };

    cycle.status = final_status.clone();
    cycle.audit_file = Some(format!("mailbox/{run_id}-audit-response.json"));
    cycle.audit_approved = Some(audit_response.approved);
    cycle.updated_at = Utc::now();
    save_cycle(&config.orchestrator_dir, &cycle)?;

    if !audit_response.approved {
        let handoff = create_auditor_to_dev_handoff(&run_id, &step_id, &audit_response);
        save_handoff(&config.orchestrator_dir, &run_id, "auditor", "dev", &handoff)?;
        sync_handoff_summary(&config.orchestrator_dir, &run_id, &handoff)?;
        db.log_event(EventType::AuditRejected, Some("auditor"), Some("dev"), Some(&format!("Auditoria REPROVADA (score {}): {}", audit_response.score, audit_response.problems.join("; "))), None, false, None).await?;
        db.log_event(EventType::HandoffCreated, Some("auditor"), Some("dev"), Some(&handoff.summary), None, false, None).await?;
        db.log_handoff(
            "auditor", "dev", "rejected", &handoff.summary,
            handoff.decisions.clone(), handoff.risks.clone(),
            handoff.open_questions.clone(), handoff.files_touched.clone(),
            &handoff.next_action
        ).await?;
        log_handoffs.push(json!(handoff));
    } else {
        db.log_event(EventType::AuditApproved, Some("auditor"), Some("orchestrator"), Some(&format!("Auditoria APROVADA (score {})", audit_response.score)), None, false, None).await?;
    }

    db.update_run_status(
        &format!("{:?}", final_status).to_lowercase(),
        Some(&patch_hash),
        Some(audit_response.approved),
        Some(audit_response.score as i64),
        None,
    ).await?;

    let log_path = config.orchestrator_dir.join("logs").join(format!("{run_id}.json"));
    write_json(
        &log_path,
        &json!({
            "timestamp": Utc::now().to_rfc3339(),
            "run_id": run_id,
            "step_id": step_id,
            "mode": if manual { "manual" } else { "auto" },
            "plan_hash": cycle.plan_hash,
            "memory_hash": cycle.memory_hash,
            "diff_hash": patch_hash,
            "dev_response": normalized_dev,
            "audit_response": audit_response,
            "handoffs": log_handoffs,
            "final_decision": format!("{:?}", final_status),
        }),
    )?;

    println!("\n{}", "══════════════════════════════════════".bold());
    println!("{}", "RESULTADO DA AUDITORIA".bold());
    println!("{}", "══════════════════════════════════════".bold());
    if audit_response.approved {
        println!("Status  : {}", "APROVADO".green().bold());
        println!("Score   : {}", audit_response.score.to_string().green());
        println!("Arquivos: {}", normalized_dev.files_touched.join(", "));
        println!(
            "Riscos  : {}",
            if normalized_dev.risks.is_empty() {
                "Nenhum crítico identificado".to_string()
            } else {
                normalized_dev.risks.join(", ")
            }
        );
        println!("{}", "══════════════════════════════════════".bold());

        // ── Apply + Commit inline ─────────────────────────────────────────────
        print_step("Digite APPLY para aplicar o patch:");
        let mut confirm = String::new();
        std::io::stdin().read_line(&mut confirm)?;
        if confirm.trim() != "APPLY" {
            print_warning("Apply cancelado. Execute `ai-orchestrator apply` quando quiser aplicar.");
            return Ok(());
        }

        git::apply_patch(&config.workspace_dir, &patch_path)?;
        print_success("Patch aplicado.");

        let commit_msg = format!(
            "[ai-orchestrator] step {step_id}: {} (score: {}, arquivos: {})",
            normalized_dev.summary,
            audit_response.score,
            normalized_dev.files_touched.join(", "),
        );
        git::add_all(&config.workspace_dir)?;
        git::commit(&config.workspace_dir, &commit_msg)?;
        print_success(&format!("Commit: {}", commit_msg.green()));

        cycle.status = CycleStatus::Applied;
        cycle.updated_at = Utc::now();
        save_cycle(&config.orchestrator_dir, &cycle)?;
        db.update_run_status("applied", Some(&patch_hash), Some(true), Some(audit_response.score as i64), None).await?;
    } else {
        println!("Status  : {}", "REPROVADO".red().bold());
        println!("Score   : {}", audit_response.score.to_string().red());
        if !audit_response.problems.is_empty() {
            println!("Problemas:");
            for p in &audit_response.problems {
                println!("  - {p}");
            }
        }
        if let Some(reason) = &audit_response.blocked_reason {
            println!("Motivo do bloqueio: {reason}");
        }
        println!("{}", "══════════════════════════════════════".bold());
        if manual {
            print_warning("Handoff auditor → dev criado. Corrija e salve nova resposta do Dev.");
        } else {
            print_warning("Handoff auditor → dev criado. Execute `run` novamente.");
        }
    }

    Ok(())
}
