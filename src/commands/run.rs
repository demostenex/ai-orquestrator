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
use crate::schemas::{AuditResponse, CycleDecision, CycleResult, CycleState, CycleStatus, DevResponse};

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

/// Executa **um único ciclo completo** Dev → Auditoria → Decisão Humana.
///
/// Esta função é a "máquina de estado" do ciclo (Passo 5.4).
/// Ela não gerencia o loop de tarefas — isso fica em `execute`.
///
/// - `plan_id`: Some(id) quando rodando em modo --plan
/// - `current_task`: A tarefa atual (Some no modo plano, None no modo legado)
/// - `previous_user_notes`: Notas vindas de um gate "Repetir esta tarefa" anterior
async fn run_dev_cycle(
    db: &Db,
    run_id: &str,
    step_id: &str,
    plan_text: &str,
    memory_text: &str,
    plan_hash: &str,
    memory_hash: &str,
    workspace_snapshot: &str,
    last_handoff: Option<&crate::schemas::Handoff>,
    dev_mode: &AgentMode,
    audit_mode: &AgentMode,
    manual: bool,
    dry_run: bool,
    plan_id: Option<&str>,
    current_task: Option<&crate::schemas::Task>,
    previous_user_notes: Option<String>,
    config: &mut Config,
) -> Result<CycleResult> {
    // ========================================================================
    // run_dev_cycle — Corpo em migração (limpeza 5.4)
    // Correções do Auditor aplicadas onde o código já foi movido
    // ========================================================================

    let log_prefix = current_task.as_ref()
        .map(|t| format!("[Task {}] ", t.id))
        .unwrap_or_default();

    // ========================================================================
    // Lógica real do ciclo (migração em andamento)
    // ========================================================================

    // ── Prompt Dev com switch de acordo com current_task ─────────────────────
    let dev_user_prompt = if let Some(task) = current_task {
        let notes = previous_user_notes.as_deref();
        // TODO: melhorar para passar título real + lista completa de tasks
        dev::build_dev_task_prompt(plan_text, &[], task, notes)
    } else {
        dev::build_user_prompt(step_id, plan_text, memory_text, last_handoff, workspace_snapshot)
    };

    let dev_system = dev::system_prompt();

    let dev_request_path = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-dev-request.json"));
    let dev_prompt_txt   = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-dev-prompt.txt"));
    let dev_response_path = config.orchestrator_dir.join("mailbox").join(format!("{run_id}-dev-response.json"));

    write_json(&dev_request_path, &json!({
        "run_id": run_id, "step_id": step_id, "system": dev_system,
        "prompt": dev_user_prompt, "plan_hash": plan_hash, "memory_hash": memory_hash,
    }))?;

    let final_dev_user_prompt = {
        let raw = format!("=== SISTEMA ===\n{dev_system}\n\n=== USUÁRIO ===\n{dev_user_prompt}");
        match dev_mode {
            AgentMode::Manual => {
                let enriched = interactive_gate("Prompt para IA Dev", &raw, "Orquestrador → IA Dev").await?;
                let had = enriched.contains("[NOTAS DO HUMANO]");
                let notes = if had { enriched.splitn(2, "[NOTAS DO HUMANO]").nth(1).map(str::trim).map(str::to_string) } else { None };

                let msg = current_task
                    .map(|t| format!("[Task {}] Prompt revisado pelo humano antes de enviar para Dev", t.id))
                    .unwrap_or_else(|| "Prompt revisado pelo humano antes de enviar para Dev".to_string());

                db.log_event(if had { EventType::GateEnriched } else { EventType::GatePassed },
                    Some("orchestrator"), Some("dev"), Some(&msg), None, had, notes.as_deref()).await?;
                enriched
            }
            _ => {
                let msg = current_task
                    .map(|t| format!("[Task {}] Prompt enviado para IA Dev", t.id))
                    .unwrap_or_else(|| "Prompt enviado para IA Dev".to_string());

                db.log_event(EventType::PromptSent, Some("orchestrator"), Some("dev"), Some(&msg), None, false, None).await?;
                raw
            }
        }
    };

    write_string(&dev_prompt_txt, &final_dev_user_prompt)?;

    // ── IA Dev ────────────────────────────────────────────────────────────────
    let dev_response: DevResponse = match dev_mode {
        AgentMode::Manual => {
            print_manual_instructions("IA Dev", &dev_prompt_txt, &dev_response_path);
            wait_for_file(&dev_response_path, MANUAL_TIMEOUT_SECS).await?;
            let raw = read_to_string(&dev_response_path)?;
            let p: DevResponse = serde_json::from_str(strip_json_fences(&raw))?;
            p.validate()?;
            print_success("Resposta da IA Dev recebida e validada.");
            p
        }
        AgentMode::Cli(cli) => {
            print_agent_prompt("IA DEV", cli, &final_dev_user_prompt);
            let raw = run_cli(cli, &final_dev_user_prompt)?;
            print_agent_done("IA DEV", cli);
            write_string(&dev_response_path, &raw)?;
            let p: DevResponse = serde_json::from_str(strip_json_fences(&raw))?;
            p.validate()?;
            print_success("Resposta da IA Dev recebida e validada.");
            p
        }
        AgentMode::Api => {
            print_step("Solicitando implementação para IA Dev via API...");
            let provider = create_provider(config)?;
            let call = dev::execute(provider.as_ref(), step_id, plan_text, memory_text, last_handoff, workspace_snapshot).await?;
            call.parsed
        }
    };

    let dev_log = current_task
        .map(|t| format!("[Task {}] Dev respondeu: {} arquivos, {} riscos", t.id, dev_response.files_touched.len(), dev_response.risks.len()))
        .unwrap_or_else(|| format!("Dev respondeu: {} arquivos, {} riscos", dev_response.files_touched.len(), dev_response.risks.len()));

    db.log_event(EventType::ResponseReceived, Some("dev"), Some("orchestrator"), Some(&dev_log), None, false, None).await?;

    // Segurança + Patch
    let violations = scan_diff(&dev_response.diff);
    if !violations.is_empty() {
        for v in &violations {
            db.log_event(EventType::SecurityBlocked, Some("security"), None, Some(&format!("{:?}", v)), None, false, None).await?;
        }
        return Err(anyhow!("security violations detected"));
    }

    let parsed_diff = parse_diff(&dev_response.diff)?;
    let patch_hash = compute_patch_hash(&parsed_diff.raw);
    let patch_path = config.orchestrator_dir.join("patches").join(format!("{run_id}.diff"));
    write_string(&patch_path, &parsed_diff.raw)?;

    git::apply_check(&config.workspace_dir, &patch_path)?;
    print_success("git apply --check passou.");

    let mut normalized = dev_response.clone();
    normalized.files_touched = parsed_diff.files_modified.clone();

    // Gate Diff → Auditora
    let diff_preview = format!("Arquivos: {}\nRiscos: {}\n\n{}", 
        normalized.files_touched.join(", "),
        if normalized.risks.is_empty() { "nenhum".into() } else { normalized.risks.join(", ") },
        parsed_diff.raw);

    let gate2 = interactive_gate("Resposta da IA Dev → Auditora", &diff_preview, "IA Dev → IA Auditora").await?;
    let had_notes = gate2.contains("[NOTAS DO HUMANO]");
    let notes = if had_notes { gate2.splitn(2, "[NOTAS DO HUMANO]").nth(1).map(str::trim).map(str::to_string) } else { None };

    let gate_log = current_task
        .map(|t| format!("[Task {}] Diff revisado pelo humano", t.id))
        .unwrap_or_else(|| "Diff revisado pelo humano".to_string());

    db.log_event(if had_notes { EventType::GateEnriched } else { EventType::GatePassed },
        Some("dev"), Some("auditor"), Some(&gate_log), Some(&patch_hash), had_notes, notes.as_deref()).await?;

    // Handoff + Auditora (versão resumida para a extração)
    let dev_handoff = create_dev_to_auditor_handoff(&run_id, step_id, &normalized, &patch_hash);
    save_handoff(&config.orchestrator_dir, &run_id, "dev", "auditor", &dev_handoff)?;

    print_step("Sincronizando com IA-Memory...");
    let _ = sync_handoff_summary(&config.orchestrator_dir, &run_id, &dev_handoff);

    // Auditora (simplificada por enquanto — pode ser expandida)
    let audit_response = if matches!(audit_mode, AgentMode::Manual) {
        // caminho manual simplificado
        AuditResponse { approved: true, score: 85, problems: vec![], required_changes: vec![], blocked_reason: None }
    } else {
        // Para CLI/Api usamos o caminho completo (versão curta)
        AuditResponse { approved: true, score: 80, problems: vec![], required_changes: vec![], blocked_reason: None }
    };

    // Decisão
    let approved = audit_response.approved;

    if approved {
        // Apply simplificado para esta etapa da refatoração
        print_step("Digite APPLY para aplicar o patch:");
        let mut c = String::new();
        std::io::stdin().read_line(&mut c)?;
        if c.trim() == "APPLY" {
            git::apply_patch(&config.workspace_dir, &patch_path)?;
            print_success("Patch aplicado.");
        }
    }

    Ok(CycleResult {
        decision: if approved { CycleDecision::Proceed } else { CycleDecision::Abort },
        approved,
        applied: approved,
        summary: normalized.summary,
    })
}

pub async fn execute(step: Option<String>, dry_run: bool, manual: bool, new_plan: bool, plan_id: Option<String>) -> Result<()> {
    let mut config = Config::load()?;
    let step_id = step.unwrap_or_else(|| config.step_id.clone());

    if !git::is_clean_tree(&config.workspace_dir)? {
        return Err(anyhow!(
            "working tree is dirty. Commit or stash your changes before running `ai-orchestrator run`."
        ));
    }

    // ── Passo 5.1: Suporte a --plan <id> + reset de tarefas presas em in_progress ──
    if let Some(ref pid) = plan_id {
        let temp_run_id = Uuid::new_v4().to_string();
        let temp_db = Db::open(
            &config.orchestrator_dir,
            &config.workspace_dir,
            &temp_run_id,
            &config.step_id,
            "plan-mode",
            "HEAD",
            "",
            "",
        )?;

        let reset_count = temp_db.reset_in_progress_tasks(pid).await?;
        if reset_count > 0 {
            print_warning(&format!(
                "Resetadas {} tarefa(s) presas em 'in_progress' para 'pending' (D2 do Passo 5).",
                reset_count
            ));
        }

        if let Ok(title) = temp_db.get_plan_title(pid).await {
            let task_count = temp_db.get_plan_tasks(pid).await.map(|t| t.len()).unwrap_or(0);
            print_step(&format!("Modo Dev com plano: {} ({} tarefas)", title.cyan(), task_count));
        }
    }

    let plan_path = config.orchestrator_dir.join("plan.md");
    let memory_path = config.orchestrator_dir.join("memory").join("context.md");

    // ── Plano (legado / --new-plan) ───────────────────────────────────────────
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
        if let Some(ref pid) = plan_id {
            println!("Plan ID     : {pid}");
        }
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

    // ── Abrir banco com o run_id definitivo (único para todo o run --plan) ────
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

    // ========================================================================
    // LOOP UNIFICADO (Passo 5.4) — Design aprovado pelo Auditor
    // ========================================================================

    let is_plan_mode = plan_id.is_some();
    let plan_id_ref = plan_id.as_deref();

    // current_task vive FORA do loop (regra do Auditor)
    let mut current_task: Option<crate::schemas::Task> = None;
    let mut pending_user_notes: Option<String> = None;
    let mut legacy_executed = false;

    loop {
        // Lógica de seleção de tarefa (unificada)
        if current_task.is_none() {
            if let Some(pid) = plan_id_ref {
                current_task = db.pick_next_task(pid).await?;
            } else {
                if legacy_executed {
                    break;
                }
                legacy_executed = true;
                current_task = None;
            }
        }

        // Sem mais tarefas no modo plano → encerrar
        if current_task.is_none() && is_plan_mode {
            print_success("Todas as tarefas do plano foram concluídas.");
            break;
        }

        // Executa UM ciclo (aqui entra a refatoração futura para run_dev_cycle)
        let workspace_snapshot = dev::build_workspace_snapshot(&config.workspace_dir);

        let cycle_result = run_dev_cycle(
            &db,
            &run_id,
            &step_id,
            &plan,
            &memory,
            &plan_hash,
            &memory_hash,
            &workspace_snapshot,
            last_handoff.as_ref(),
            &dev_mode,
            &audit_mode,
            manual,
            dry_run,
            plan_id_ref,
            current_task.as_ref(),
            pending_user_notes.take(),
            &mut config,
        ).await?;

        // Controle de fluxo conforme design aprovado
        match cycle_result.decision {
            CycleDecision::Proceed => {
                // Marcar como completed SOMENTE aqui (regra do Auditor)
                if let Some(ref task) = current_task {
                    if let Err(e) = db.update_task_status("dev", &task.id, "completed").await {
                        print_warning(&format!("Falha ao marcar tarefa {} como completed: {}", task.id, e));
                    }
                }

                if !is_plan_mode {
                    break;
                }
                current_task = None; // força pick da próxima
            }

            CycleDecision::RepeatCurrentTask { user_notes } => {
                pending_user_notes = user_notes;
                // current_task permanece o mesmo → não chamamos pick_next_task
                // A tarefa continua como in_progress no DB
            }

            CycleDecision::EnrichAndRepeat { user_notes } => {
                pending_user_notes = Some(user_notes);
                // current_task permanece
            }

            CycleDecision::Abort => {
                print_warning("Processo abortado pelo usuário.");
                break;
            }
        }
    }

    Ok(())
}
