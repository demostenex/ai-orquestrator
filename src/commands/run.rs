use anyhow::{anyhow, Result};
use chrono::Utc;
use colored::Colorize;
use serde_json::json;
use uuid::Uuid;

use crate::agents::{auditor, dev};
use crate::commands::apply::apply_patch_rigorous;
use crate::commands::{
    interactive_gate, print_manual_instructions, print_step, print_success, print_warning,
    read_to_string, save_cycle, wait_for_file, write_json, write_string,
};
use crate::core::cli_runner::{is_cli_available, run_cli, select_cli};
use crate::core::compute_sha256;
use crate::core::config::{Config, StoredConfig};
use crate::core::db::{Db, EventType};
use crate::core::git;
use crate::core::handoff::{
    create_auditor_to_dev_handoff, create_dev_to_auditor_handoff, load_last_handoff, save_handoff,
};
use crate::core::memory::sync_handoff_summary;
use crate::core::patch::{compute_patch_hash, parse_diff};
use crate::core::security::scan_diff;
use crate::core::strip_json_fences;
use crate::providers::create_provider;
use crate::schemas::{
    AuditResponse, CycleDecision, CycleResult, CycleState, CycleStatus, DevResponse,
};

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
    println!(
        "{}",
        format!("⏳ {role} ({cli}) TRABALHANDO...").bold().yellow()
    );
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
    #[allow(dead_code)]
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
            println!(
                "{}",
                format!("⚠ CLI '{cli}' salvo no config não encontrado. Selecionando novamente...")
                    .yellow()
            );
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
#[allow(clippy::too_many_arguments)]
async fn run_dev_cycle(
    db: &Db,
    run_id: &str,
    step_id: &str,
    plan_text: &str,
    memory_text: &str,
    plan_hash: &str,
    memory_hash: &str,
    base_commit: &str,
    workspace_snapshot: &str,
    last_handoff: Option<&crate::schemas::Handoff>,
    dev_mode: &AgentMode,
    audit_mode: &AgentMode,
    _manual: bool,
    _dry_run: bool,
    plan_id: Option<&str>,
    current_task: Option<&crate::schemas::Task>,
    previous_user_notes: Option<String>,
    config: &mut Config,
) -> Result<CycleResult> {
    // ========================================================================
    // run_dev_cycle — Corpo em migração (limpeza 5.4)
    // Correções do Auditor aplicadas onde o código já foi movido
    // ========================================================================

    // ========================================================================
    // Lógica real do ciclo (migração em andamento)
    // ========================================================================

    // ── Prompt Dev com contexto rico do plano (Passo 5.6 - Decisão D1) ───────
    let dev_user_prompt = if let Some(task) = current_task {
        let notes = previous_user_notes.as_deref();

        if let Some(pid) = plan_id {
            // Buscar título e lista completa de tarefas do plano
            let plan_title = db.get_plan_title(pid).await?;
            let all_tasks = db.get_plan_tasks(pid).await?;

            // Log de transparência (recomendado pelo Auditor)
            db.log_event(
                EventType::PromptSent,
                Some("orchestrator"),
                Some("dev"),
                Some(&format!(
                    "[Task {}] Buscando contexto rico ({} tarefas) do plano no banco",
                    task.id,
                    all_tasks.len()
                )),
                None,
                false,
                None,
            )
            .await?;

            dev::build_dev_task_prompt(&plan_title, &all_tasks, task, notes)
        } else {
            // Fallback defensivo (modo plano sem plan_id)
            dev::build_dev_task_prompt(plan_text, &[], task, notes)
        }
    } else {
        dev::build_user_prompt(
            step_id,
            plan_text,
            memory_text,
            last_handoff,
            workspace_snapshot,
        )
    };

    let dev_system = dev::system_prompt();

    let dev_request_path = config
        .orchestrator_dir
        .join("mailbox")
        .join(format!("{run_id}-dev-request.json"));
    let dev_prompt_txt = config
        .orchestrator_dir
        .join("mailbox")
        .join(format!("{run_id}-dev-prompt.txt"));
    let dev_response_path = config
        .orchestrator_dir
        .join("mailbox")
        .join(format!("{run_id}-dev-response.json"));
    let audit_request_path = config
        .orchestrator_dir
        .join("mailbox")
        .join(format!("{run_id}-audit-request.json"));
    let audit_prompt_txt = config
        .orchestrator_dir
        .join("mailbox")
        .join(format!("{run_id}-audit-prompt.txt"));
    let audit_response_path = config
        .orchestrator_dir
        .join("mailbox")
        .join(format!("{run_id}-audit-response.json"));

    write_json(
        &dev_request_path,
        &json!({
            "run_id": run_id, "step_id": step_id, "system": dev_system,
            "prompt": dev_user_prompt, "plan_hash": plan_hash, "memory_hash": memory_hash,
        }),
    )?;

    let final_dev_user_prompt = {
        let raw = format!("=== SISTEMA ===\n{dev_system}\n\n=== USUÁRIO ===\n{dev_user_prompt}");
        match dev_mode {
            AgentMode::Manual => {
                let enriched =
                    interactive_gate("Prompt para IA Dev", &raw, "Orquestrador → IA Dev").await?;
                let had = enriched.contains("[NOTAS DO HUMANO]");
                let notes = if had {
                    enriched
                        .split_once("[NOTAS DO HUMANO]")
                        .map(|(_, notes)| notes)
                        .map(str::trim)
                        .map(str::to_string)
                } else {
                    None
                };

                let msg = current_task
                    .map(|t| {
                        format!(
                            "[Task {}] Prompt revisado pelo humano antes de enviar para Dev",
                            t.id
                        )
                    })
                    .unwrap_or_else(|| {
                        "Prompt revisado pelo humano antes de enviar para Dev".to_string()
                    });

                db.log_event(
                    if had {
                        EventType::GateEnriched
                    } else {
                        EventType::GatePassed
                    },
                    Some("orchestrator"),
                    Some("dev"),
                    Some(&msg),
                    None,
                    had,
                    notes.as_deref(),
                )
                .await?;
                enriched
            }
            _ => {
                let msg = current_task
                    .map(|t| format!("[Task {}] Prompt enviado para IA Dev", t.id))
                    .unwrap_or_else(|| "Prompt enviado para IA Dev".to_string());

                db.log_event(
                    EventType::PromptSent,
                    Some("orchestrator"),
                    Some("dev"),
                    Some(&msg),
                    None,
                    false,
                    None,
                )
                .await?;
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
            let raw = run_cli(
                cli,
                &final_dev_user_prompt,
                None::<crate::core::stream::LogTx>,
            )?;
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
            let call = dev::execute(
                provider.as_ref(),
                step_id,
                plan_text,
                memory_text,
                last_handoff,
                workspace_snapshot,
            )
            .await?;
            call.parsed
        }
    };

    let dev_log = current_task
        .map(|t| {
            format!(
                "[Task {}] Dev respondeu: {} arquivos, {} riscos",
                t.id,
                dev_response.files_touched.len(),
                dev_response.risks.len()
            )
        })
        .unwrap_or_else(|| {
            format!(
                "Dev respondeu: {} arquivos, {} riscos",
                dev_response.files_touched.len(),
                dev_response.risks.len()
            )
        });

    db.log_event(
        EventType::ResponseReceived,
        Some("dev"),
        Some("orchestrator"),
        Some(&dev_log),
        None,
        false,
        None,
    )
    .await?;

    // Segurança + Patch
    let violations = scan_diff(&dev_response.diff);
    if !violations.is_empty() {
        for v in &violations {
            db.log_event(
                EventType::SecurityBlocked,
                Some("security"),
                None,
                Some(&format!("{:?}", v)),
                None,
                false,
                None,
            )
            .await?;
        }
        return Err(anyhow!("security violations detected"));
    }

    let parsed_diff = parse_diff(&dev_response.diff)?;
    let patch_hash = compute_patch_hash(&parsed_diff.raw);
    let patch_path = config
        .orchestrator_dir
        .join("patches")
        .join(format!("{run_id}.diff"));
    write_string(&patch_path, &parsed_diff.raw)?;

    git::apply_check(&config.workspace_dir, &patch_path)?;
    print_success("git apply --check passou.");

    let mut normalized = dev_response.clone();
    normalized.files_touched = parsed_diff.files_modified.clone();

    // Gate Diff → Auditora
    let diff_preview = format!(
        "Arquivos: {}\nRiscos: {}\n\n{}",
        normalized.files_touched.join(", "),
        if normalized.risks.is_empty() {
            "nenhum".into()
        } else {
            normalized.risks.join(", ")
        },
        parsed_diff.raw
    );

    let gate2 = interactive_gate(
        "Resposta da IA Dev → Auditora",
        &diff_preview,
        "IA Dev → IA Auditora",
    )
    .await?;
    let had_notes = gate2.contains("[NOTAS DO HUMANO]");
    let notes = if had_notes {
        gate2
            .split_once("[NOTAS DO HUMANO]")
            .map(|(_, notes)| notes)
            .map(str::trim)
            .map(str::to_string)
    } else {
        None
    };

    let gate_log = current_task
        .map(|t| format!("[Task {}] Diff revisado pelo humano", t.id))
        .unwrap_or_else(|| "Diff revisado pelo humano".to_string());

    db.log_event(
        if had_notes {
            EventType::GateEnriched
        } else {
            EventType::GatePassed
        },
        Some("dev"),
        Some("auditor"),
        Some(&gate_log),
        Some(&patch_hash),
        had_notes,
        notes.as_deref(),
    )
    .await?;

    // Handoff + Auditora (versão resumida para a extração)
    let dev_handoff = create_dev_to_auditor_handoff(run_id, step_id, &normalized, &patch_hash);
    save_handoff(
        &config.orchestrator_dir,
        run_id,
        "dev",
        "auditor",
        &dev_handoff,
    )?;

    print_step("Sincronizando com IA-Memory...");
    let _ = sync_handoff_summary(&config.orchestrator_dir, run_id, &dev_handoff);

    let audit_user_prompt = auditor::build_user_prompt(
        step_id,
        plan_text,
        memory_text,
        Some(&dev_handoff),
        &parsed_diff.raw,
        &parsed_diff.files_modified,
        "git apply --check: success",
    );
    let final_audit_prompt = format!(
        "=== SISTEMA ===\n{}\n\n=== USUÁRIO ===\n{}",
        auditor::system_prompt(),
        audit_user_prompt
    );

    write_json(
        &audit_request_path,
        &json!({
            "run_id": run_id,
            "step_id": step_id,
            "system": auditor::system_prompt(),
            "prompt": audit_user_prompt,
            "patch_hash": patch_hash,
        }),
    )?;
    write_string(&audit_prompt_txt, &final_audit_prompt)?;

    let audit_response: AuditResponse = match audit_mode {
        AgentMode::Manual => {
            print_manual_instructions("IA Auditora", &audit_prompt_txt, &audit_response_path);
            wait_for_file(&audit_response_path, MANUAL_TIMEOUT_SECS).await?;
            let raw = read_to_string(&audit_response_path)?;
            let p: AuditResponse = serde_json::from_str(strip_json_fences(&raw))?;
            p.validate()?;
            print_success("Resposta da IA Auditora recebida e validada.");
            p
        }
        AgentMode::Cli(cli) => {
            print_agent_prompt("IA AUDITORA", cli, &final_audit_prompt);
            let raw = run_cli(cli, &final_audit_prompt, None::<crate::core::stream::LogTx>)?;
            print_agent_done("IA AUDITORA", cli);
            write_string(&audit_response_path, &raw)?;
            let p: AuditResponse = serde_json::from_str(strip_json_fences(&raw))?;
            p.validate()?;
            print_success("Resposta da IA Auditora recebida e validada.");
            p
        }
        AgentMode::Api => {
            print_step("Solicitando auditoria via API...");
            let provider = create_provider(config)?;
            let call = auditor::execute(
                provider.as_ref(),
                step_id,
                plan_text,
                memory_text,
                Some(&dev_handoff),
                &parsed_diff.raw,
                &parsed_diff.files_modified,
                "git apply --check: success",
            )
            .await?;
            write_json(
                &audit_response_path,
                &json!({
                    "parsed": call.parsed,
                    "raw_text": call.provider_response.text,
                }),
            )?;
            call.parsed
        }
    };
    let audit_json = json!({ "parsed": &audit_response });
    write_json(&audit_response_path, &audit_json)?;

    db.log_event(
        if audit_response.approved {
            EventType::AuditApproved
        } else {
            EventType::AuditRejected
        },
        Some("auditor"),
        Some(if audit_response.approved {
            "human"
        } else {
            "dev"
        }),
        Some(&format!(
            "Auditoria {} com score {}",
            if audit_response.approved {
                "aprovada"
            } else {
                "reprovada"
            },
            audit_response.score
        )),
        Some(&patch_hash),
        false,
        None,
    )
    .await?;

    if !audit_response.approved {
        let handoff = create_auditor_to_dev_handoff(run_id, step_id, &audit_response);
        save_handoff(&config.orchestrator_dir, run_id, "auditor", "dev", &handoff)?;
        let _ = sync_handoff_summary(&config.orchestrator_dir, run_id, &handoff);
    }

    // Decisão
    let approved = audit_response.approved;
    let now = Utc::now();
    let mut cycle = CycleState {
        run_id: run_id.to_string(),
        step_id: step_id.to_string(),
        status: if approved {
            CycleStatus::Approved
        } else {
            CycleStatus::Rejected
        },
        base_commit: base_commit.to_string(),
        plan_hash: plan_hash.to_string(),
        memory_hash: memory_hash.to_string(),
        patch_file: Some(
            patch_path
                .strip_prefix(&config.orchestrator_dir)
                .unwrap_or(&patch_path)
                .to_string_lossy()
                .into_owned(),
        ),
        patch_hash: Some(patch_hash.clone()),
        audit_file: Some(
            audit_response_path
                .strip_prefix(&config.orchestrator_dir)
                .unwrap_or(&audit_response_path)
                .to_string_lossy()
                .into_owned(),
        ),
        audit_approved: Some(approved),
        created_at: now,
        updated_at: now,
    };
    save_cycle(&config.orchestrator_dir, &cycle)?;

    if approved {
        apply_patch_rigorous(config, &mut cycle, &patch_path, &audit_json, false).await?;
        print_success("Patch aplicado.");
    }

    Ok(CycleResult {
        decision: if approved {
            CycleDecision::Proceed
        } else {
            CycleDecision::Abort
        },
        approved,
        applied: approved,
        summary: normalized.summary,
    })
}

pub async fn execute(
    step: Option<String>,
    dry_run: bool,
    manual: bool,
    new_plan: bool,
    plan_id: Option<String>,
) -> Result<()> {
    let mut config = Config::load()?;
    let step_id = step.unwrap_or_else(|| config.step_id.clone());
    let base_commit = git::ensure_repository(&config.workspace_dir)?;

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
            let task_count = temp_db
                .get_plan_tasks(pid)
                .await
                .map(|t| t.len())
                .unwrap_or(0);
            print_step(&format!(
                "Modo Dev com plano: {} ({} tarefas)",
                title.cyan(),
                task_count
            ));
        }
    }

    let plan_path = config.orchestrator_dir.join("plan.md");
    let memory_path = config.orchestrator_dir.join("memory").join("context.md");

    // ── Plano (legado / --new-plan) ───────────────────────────────────────────
    let plan = if new_plan || !plan_path.exists() {
        if new_plan {
            print_warning("--new-plan está depreciado (decisão O1 do Passo 4).");
            println!(
                "  Use {} no futuro para criar planos.",
                "ai-orchestrator plan new".cyan()
            );
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
            if line.trim().is_empty() {
                break;
            }
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

    let dev_cli_name = match &dev_mode {
        AgentMode::Cli(c) => Some(c.clone()),
        _ => None,
    };
    let audit_cli_name = match &audit_mode {
        AgentMode::Cli(c) => Some(c.clone()),
        _ => None,
    };

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

    println!(
        "  Dev      : {}",
        match &dev_mode {
            AgentMode::Cli(c) => c.green().to_string(),
            AgentMode::Api => "API".cyan().to_string(),
            AgentMode::Manual => "manual".yellow().to_string(),
        }
    );
    println!(
        "  Auditora : {}",
        match &audit_mode {
            AgentMode::Cli(c) => c.green().to_string(),
            AgentMode::Api => "API".cyan().to_string(),
            AgentMode::Manual => "manual".yellow().to_string(),
        }
    );
    println!("{}", "══════════════════════════════════════".bold());

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

        // Sem mais tarefas no modo plano → encerrar com resumo
        if current_task.is_none() && is_plan_mode {
            println!(
                "\n{}",
                "══════════════════════════════════════════════════════════".bold()
            );
            println!("{}", "✅ PLANO CONCLUÍDO".bold().green());
            println!(
                "{}",
                "══════════════════════════════════════════════════════════".bold()
            );
            println!("Todas as tarefas foram finalizadas com sucesso.");
            // TODO futuro: mostrar estatísticas (tarefas concluídas, repetições, etc.)
            println!(
                "{}",
                "══════════════════════════════════════════════════════════".bold()
            );
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
            &base_commit,
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
        )
        .await?;

        // ============================================================
        // PASSO 5.5 - Gate Inter-Tarefa (decisão humana tem precedência)
        // ============================================================
        let human_decision = if is_plan_mode {
            crate::commands::interactive_inter_task_gate(current_task.as_ref(), &cycle_result)
                .await?
        } else {
            // No modo legado, respeitamos diretamente o resultado do ciclo
            cycle_result.decision.clone()
        };

        // Controle de fluxo baseado na decisão do humano
        match human_decision {
            CycleDecision::Proceed => {
                // Marcar como completed SOMENTE aqui (regra do Auditor)
                if let Some(ref task) = current_task {
                    if let Err(e) = db.update_task_status("dev", &task.id, "completed").await {
                        print_warning(&format!(
                            "Falha ao marcar tarefa {} como completed: {}",
                            task.id, e
                        ));
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

/// Versão nativa do loop dev para o TUI (background thread).
/// Substitui `execute` quando chamado do Ratatui — sem println!, sem inquire, sem stdin.
pub async fn execute_tui(
    plan_id: String,
    cli_dev: String,
    cli_audit: Option<String>,
    log_tx: crate::core::stream::LogTx,
    gate_rx: crate::core::stream::GateRx,
) -> anyhow::Result<()> {
    use crate::core::stream::{GateDecision, LogEvent};

    let send = |msg: String| {
        let _ = log_tx.send(LogEvent::Line(msg));
    };
    // Variante com origem estruturada: ancora o pane do agente no TUI sem
    // heurística. O stdout cru das CLIs (via run_cli) continua como `Line` e
    // herda o último autor ancorado por aqui.
    let send_agent = |msg: String, origin: crate::core::stream::LineOrigin| {
        let _ = log_tx.send(LogEvent::AgentLine { text: msg, origin });
    };
    let send_gate = |content: String, gate_type: &str| {
        let _ = log_tx.send(LogEvent::GateNeeded {
            content,
            gate_type: gate_type.to_string(),
        });
    };
    let recv_gate = || {
        gate_rx
            .recv_timeout(std::time::Duration::from_secs(7200))
            .unwrap_or(GateDecision::Abort)
    };

    let config = Config::load()?;
    let cli_audit = cli_audit.ok_or_else(|| anyhow::anyhow!("CLI da Auditora é obrigatório"))?;
    let base_commit = git::ensure_repository(&config.workspace_dir)?;

    if !git::is_clean_tree(&config.workspace_dir)? {
        return Err(anyhow::anyhow!(
            "working tree dirty — commit ou stash antes de rodar"
        ));
    }

    let memory_path = config.orchestrator_dir.join("memory").join("context.md");
    let memory = std::fs::read_to_string(&memory_path).unwrap_or_default();
    let plan_path = config.orchestrator_dir.join("plan.md");
    let plan = std::fs::read_to_string(&plan_path).unwrap_or_default();
    let plan_hash = compute_sha256(&plan);
    let memory_hash = compute_sha256(&memory);
    let run_id = uuid::Uuid::new_v4().to_string();

    let db = Db::open(
        &config.orchestrator_dir,
        &config.workspace_dir,
        &run_id,
        &config.step_id,
        "auto",
        &base_commit,
        &plan_hash,
        &memory_hash,
    )?;

    let reset = db.reset_in_progress_tasks(&plan_id).await?;
    if reset > 0 {
        send(format!("⚠ {} tarefa(s) in_progress → pending", reset));
    }

    let plan_title = db.get_plan_title(&plan_id).await.unwrap_or_default();
    send(format!("📋 Plano: {} | Dev: {}", plan_title, cli_dev));

    let mut pending_notes: Option<String> = None;

    loop {
        let task = db.pick_next_task(&plan_id).await?;
        if task.is_none() {
            send("✅ Todas as tarefas concluídas!".to_string());
            let _ = log_tx.send(LogEvent::Done);
            return Ok(());
        }
        let task = task.unwrap();
        send(format!("\n══ Tarefa: {} ══", task.description));

        let all_tasks = db.get_plan_tasks(&plan_id).await.unwrap_or_default();
        let dev_user_prompt =
            dev::build_dev_task_prompt(&plan_title, &all_tasks, &task, pending_notes.as_deref());
        pending_notes = None;

        let dev_system = dev::system_prompt();
        let final_prompt =
            format!("=== SISTEMA ===\n{dev_system}\n\n=== USUÁRIO ===\n{dev_user_prompt}");

        db.log_event(
            crate::core::db::EventType::PromptSent,
            Some("orchestrator"),
            Some("dev"),
            Some(&format!("[Task {}] Prompt enviado", task.id)),
            None,
            false,
            None,
        )
        .await?;

        send_agent(
            format!("⏳ IA Dev ({}) trabalhando...", cli_dev),
            crate::core::stream::LineOrigin::Dev,
        );

        let raw = {
            let cli = cli_dev.clone();
            let p = final_prompt.clone();
            let tx = log_tx.clone();
            tokio::task::spawn_blocking(move || run_cli(&cli, &p, Some(tx))).await??
        };

        send_agent(
            "✔ Dev concluído. Processando resposta...".to_string(),
            crate::core::stream::LineOrigin::Dev,
        );

        // Parse e validação
        let dev_response: crate::schemas::DevResponse =
            match serde_json::from_str(strip_json_fences(&raw)) {
                Ok(r) => r,
                Err(e) => {
                    send(format!("❌ JSON inválido: {}", e));
                    db.update_task_status("dev", &task.id, "pending").await.ok();
                    continue;
                }
            };

        // Security scan
        let violations = crate::core::security::scan_diff(&dev_response.diff);
        if !violations.is_empty() {
            for v in &violations {
                send(format!("🚫 Segurança: {:?}", v));
                db.log_event(
                    crate::core::db::EventType::SecurityBlocked,
                    Some("security"),
                    None,
                    Some(&format!("{:?}", v)),
                    None,
                    false,
                    None,
                )
                .await?;
            }
            send(
                "🛑 Tarefa bloqueada: violações de segurança não podem ser ignoradas.".to_string(),
            );
            db.update_task_status("dev", &task.id, "blocked").await.ok();
            continue;
        }

        // Parse diff
        let parsed_diff = match crate::core::patch::parse_diff(&dev_response.diff) {
            Ok(d) => d,
            Err(e) => {
                send(format!("❌ Diff inválido: {}", e));
                db.update_task_status("dev", &task.id, "pending").await.ok();
                continue;
            }
        };

        let patch_hash = crate::core::patch::compute_patch_hash(&parsed_diff.raw);
        let patch_path = config.orchestrator_dir.join("patches").join(format!(
            "{}-{}.diff",
            run_id,
            &task.id[..8]
        ));
        std::fs::write(&patch_path, &parsed_diff.raw)?;

        if let Err(e) = git::apply_check(&config.workspace_dir, &patch_path) {
            send(format!("❌ git apply --check falhou: {}", e));
            db.update_task_status("dev", &task.id, "pending").await.ok();
            continue;
        }
        send("✔ git apply --check passou.".to_string());
        db.log_event(
            crate::core::db::EventType::GitCheckPassed,
            Some("dev"),
            Some("auditor"),
            Some(&format!("[Task {}] Check passou", task.id)),
            Some(&patch_hash),
            false,
            None,
        )
        .await?;

        // Gate: revisão do diff
        let diff_lines: Vec<&str> = parsed_diff.raw.lines().collect();
        let preview_lines = diff_lines
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let diff_preview = format!(
            "Arquivos: {}\nRiscos: {}\n\n{}{}",
            parsed_diff.files_modified.join(", "),
            if dev_response.risks.is_empty() {
                "nenhum".to_string()
            } else {
                dev_response.risks.join(", ")
            },
            preview_lines,
            if diff_lines.len() > 30 {
                format!("\n... (+{} linhas)", diff_lines.len() - 30)
            } else {
                String::new()
            }
        );
        send_gate(diff_preview, "diff_review");

        match recv_gate() {
            GateDecision::Continue | GateDecision::Review => {}
            GateDecision::Enrich(notes) => {
                pending_notes = Some(notes);
                db.update_task_status("dev", &task.id, "pending").await.ok();
                continue;
            }
            GateDecision::Abort | GateDecision::Finalize => {
                send("❌ Diff rejeitado.".to_string());
                db.update_task_status("dev", &task.id, "pending").await.ok();
                continue;
            }
        }

        let gate_event_id = db
            .log_event(
                crate::core::db::EventType::GatePassed,
                Some("dev"),
                Some("auditor"),
                Some(&format!("[Task {}] Diff aprovado", task.id)),
                Some(&patch_hash),
                false,
                None,
            )
            .await?;

        // Escreve handoff no ai-memory e salva o "recibo" (URL) no evento
        {
            let wiki_path = format!("handoffs/run_{}/task_{}.md", &run_id[..8], &task.id[..8]);
            let title = format!("[Dev] {} — {}", &task.id[..8], task.description);
            let body = format!(
                "# Dev Handoff\n\n**Task:** {}\n**Run:** {}\n**Arquivos:** {}\n**Riscos:** {}\n\n## Diff\n```diff\n{}\n```",
                task.description, run_id,
                parsed_diff.files_modified.join(", "),
                dev_response.risks.join(", "),
                if parsed_diff.raw.len() > 2000 { format!("{}...", &parsed_diff.raw[..2000]) } else { parsed_diff.raw.clone() }
            );
            if let Some(url) = crate::core::memory::write_to_ai_memory(&wiki_path, &title, &body) {
                db.update_event_memory_url(gate_event_id, &url).await.ok();
                send(format!("🔗 Handoff registrado: {}", url));
            }
        }

        let audit_prompt = {
            let last_handoff = load_last_handoff(&config.orchestrator_dir).ok().flatten();
            let user_prompt = auditor::build_user_prompt(
                &config.step_id,
                &plan,
                &memory,
                last_handoff.as_ref(),
                &parsed_diff.raw,
                &parsed_diff.files_modified,
                "git apply --check: success",
            );
            format!(
                "=== SISTEMA ===\n{}\n\n=== USUÁRIO ===\n{}",
                auditor::system_prompt(),
                user_prompt
            )
        };

        send_agent(
            format!("⏳ IA Auditora ({}) trabalhando...", cli_audit),
            crate::core::stream::LineOrigin::Auditor,
        );
        let raw_audit = {
            let cli = cli_audit.clone();
            let p = audit_prompt;
            let tx = log_tx.clone();
            tokio::task::spawn_blocking(move || run_cli(&cli, &p, Some(tx))).await??
        };

        let audit_response: AuditResponse =
            match serde_json::from_str(strip_json_fences(&raw_audit)) {
                Ok(response) => response,
                Err(e) => {
                    send(format!("❌ JSON inválido da Auditora: {}", e));
                    db.update_task_status("dev", &task.id, "pending").await.ok();
                    continue;
                }
            };

        if let Err(e) = audit_response.validate() {
            send(format!("❌ Resposta inválida da Auditora: {}", e));
            db.update_task_status("dev", &task.id, "pending").await.ok();
            continue;
        }
        let audit_json = json!({ "parsed": &audit_response });
        let audit_response_path = config.orchestrator_dir.join("mailbox").join(format!(
            "{}-{}-audit-response.json",
            run_id,
            &task.id[..8]
        ));
        write_json(&audit_response_path, &audit_json)?;

        if !audit_response.approved {
            db.log_event(
                crate::core::db::EventType::AuditRejected,
                Some("auditor"),
                Some("dev"),
                Some(&format!(
                    "[Task {}] Auditoria reprovada: {}",
                    task.id,
                    audit_response.problems.join("; ")
                )),
                Some(&patch_hash),
                false,
                None,
            )
            .await?;

            let handoff = create_auditor_to_dev_handoff(&run_id, &config.step_id, &audit_response);
            save_handoff(
                &config.orchestrator_dir,
                &run_id,
                "auditor",
                "dev",
                &handoff,
            )?;
            let _ = sync_handoff_summary(&config.orchestrator_dir, &run_id, &handoff);

            send(format!(
                "❌ Auditoria reprovada (score {}). Correções: {}",
                audit_response.score,
                if audit_response.required_changes.is_empty() {
                    "(não informado)".to_string()
                } else {
                    audit_response.required_changes.join("; ")
                }
            ));
            db.update_task_status("dev", &task.id, "pending").await.ok();
            continue;
        }

        db.log_event(
            crate::core::db::EventType::AuditApproved,
            Some("auditor"),
            Some("human"),
            Some(&format!(
                "[Task {}] Auditoria aprovada com score {}",
                task.id, audit_response.score
            )),
            Some(&patch_hash),
            false,
            None,
        )
        .await?;
        send(format!(
            "✔ Auditoria aprovada (score {}).",
            audit_response.score
        ));

        // Gate: aplicar patch
        send_gate(
            format!(
                "Aplicar patch?\n\nArquivos: {}\nHash: {}",
                parsed_diff.files_modified.join(", "),
                &patch_hash[..16]
            ),
            "apply",
        );

        let applied = match recv_gate() {
            GateDecision::Continue | GateDecision::Review => {
                let now = Utc::now();
                let mut cycle = CycleState {
                    run_id: run_id.to_string(),
                    step_id: config.step_id.clone(),
                    status: CycleStatus::Approved,
                    base_commit: base_commit.clone(),
                    plan_hash: plan_hash.clone(),
                    memory_hash: memory_hash.clone(),
                    patch_file: Some(
                        patch_path
                            .strip_prefix(&config.orchestrator_dir)
                            .unwrap_or(&patch_path)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    patch_hash: Some(patch_hash.clone()),
                    audit_file: Some(
                        audit_response_path
                            .strip_prefix(&config.orchestrator_dir)
                            .unwrap_or(&audit_response_path)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    audit_approved: Some(true),
                    created_at: now,
                    updated_at: now,
                };
                save_cycle(&config.orchestrator_dir, &cycle)?;
                apply_patch_rigorous(&config, &mut cycle, &patch_path, &audit_json, true).await?;
                send("✔ Patch aplicado.".to_string());
                true
            }
            _ => {
                send("⏭ Patch não aplicado (skipped).".to_string());
                false
            }
        };

        // Gate inter-tarefa
        let inter_info = format!(
            "Tarefa: {}\nStatus: {}\nPatch: {}",
            task.description,
            if applied { "aplicado" } else { "skipped" },
            &patch_hash[..16]
        );
        send_gate(inter_info, "inter_task");

        match recv_gate() {
            GateDecision::Continue | GateDecision::Review => {
                db.update_task_status("dev", &task.id, "completed").await?;
                send(format!("✅ '{}' concluída.", task.description));
            }
            GateDecision::Enrich(notes) => {
                pending_notes = Some(notes);
                db.update_task_status("dev", &task.id, "pending").await.ok();
                send("↩ Repetindo tarefa com notas...".to_string());
            }
            GateDecision::Abort | GateDecision::Finalize => {
                send("🛑 Dev Mode abortado pelo usuário.".to_string());
                let _ = log_tx.send(LogEvent::Done);
                return Ok(());
            }
        }
    }
}
