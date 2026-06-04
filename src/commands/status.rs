use anyhow::Result;
use colored::Colorize;
use regex::Regex;
use serde_json::json;

use crate::commands::{load_cycle, now_string, read_to_string, status_label};
use crate::core::compute_sha256;
use crate::core::config::Config;
use crate::core::db::Db;
use crate::core::git;
use crate::core::handoff::load_last_handoff_with_meta;
use crate::core::memory::load_last_sync;
use crate::schemas::CycleStatus;

pub async fn execute(
    as_json: bool,
    short: bool,
    history: bool,
    run_id: Option<String>,
) -> Result<()> {
    let config = match Config::load() {
        Ok(config) => config,
        Err(_) => {
            println!("Run `ai-orchestrator init` first");
            return Ok(());
        }
    };

    // ── Histórico de runs ─────────────────────────────────────────────────────
    if let Some(rid) = run_id {
        let events = Db::list_events_for_run(config.orchestrator_dir.clone(), rid.clone()).await?;
        if events.is_empty() {
            println!("Nenhum evento encontrado para run {rid}");
        } else {
            println!("{}", "══════════════════════════════════════".bold());
            println!("{}", format!("TRILHA DE AUDITORIA — run {rid}").bold());
            println!("{}", "══════════════════════════════════════".bold());
            for ev in &events {
                let enriched_marker = if ev.enriched_by_human {
                    " [HUMANO]".yellow().to_string()
                } else {
                    String::new()
                };
                println!(
                    "  {:>3}. {} {:>20} → {:<20} {}{}",
                    ev.sequence,
                    &ev.timestamp[..19],
                    ev.from_agent.as_deref().unwrap_or("-"),
                    ev.to_agent.as_deref().unwrap_or("-"),
                    ev.event_type.cyan(),
                    enriched_marker,
                );
                if let Some(summary) = &ev.content_summary {
                    println!("       {}", summary.dimmed());
                }
                if let Some(notes) = &ev.human_notes {
                    println!("       {} {}", "Notas:".yellow(), notes);
                }
            }
            println!("{}", "══════════════════════════════════════".bold());
        }
        return Ok(());
    }

    if history {
        let runs = Db::list_recent_runs(
            config.orchestrator_dir.clone(),
            config.workspace_dir.clone(),
            20,
        )
        .await?;
        if runs.is_empty() {
            println!("Nenhum run encontrado para este projeto.");
        } else {
            println!("{}", "══════════════════════════════════════".bold());
            println!("{}", format!("HISTÓRICO — {}", runs[0].project_name).bold());
            println!("{}", "══════════════════════════════════════".bold());
            println!(
                "  {:>24}  {:>8}  {:>12}  {:>8}  run_id",
                "criado em", "modo", "status", "score"
            );
            println!(
                "  {}  {}  {}  {}  {}",
                "-".repeat(24),
                "-".repeat(8),
                "-".repeat(12),
                "-".repeat(8),
                "-".repeat(36)
            );
            for r in &runs {
                let status_colored = match r.status.as_str() {
                    "approved" | "applied" => r.status.green().to_string(),
                    "rejected" | "security_blocked" => r.status.red().to_string(),
                    _ => r.status.normal().to_string(),
                };
                let score = r
                    .audit_score
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".to_string());
                println!(
                    "  {:>24}  {:>8}  {:>12}  {:>8}  {}",
                    &r.created_at[..19],
                    r.mode,
                    status_colored,
                    score,
                    r.id
                );
            }
            println!("{}", "══════════════════════════════════════".bold());
            println!("Use --run-id <id> para ver a trilha completa de um run.");
        }
        return Ok(());
    }

    let cycle = load_cycle(&config.orchestrator_dir)?;
    let plan_path = config.orchestrator_dir.join("plan.md");
    let memory_path = config.orchestrator_dir.join("memory").join("context.md");
    let plan = read_to_string(&plan_path)?;
    let memory = read_to_string(&memory_path)?;
    let plan_hash = compute_sha256(&plan);
    let memory_hash = compute_sha256(&memory);
    let steps_count = count_steps(&plan);
    let branch =
        git::current_branch(&config.workspace_dir).unwrap_or_else(|_| "unknown".to_string());
    let clean = git::is_clean_tree(&config.workspace_dir).unwrap_or(false);
    let last_commit =
        git::last_commit_summary(&config.workspace_dir).unwrap_or_else(|_| "unknown".to_string());
    let patches = collect_patches(&config, &cycle)?;
    let last_handoff = load_last_handoff_with_meta(&config.orchestrator_dir)?;
    let last_sync = load_last_sync(&config.orchestrator_dir)?;
    let last_audit = cycle
        .audit_file
        .as_ref()
        .and_then(|path| read_to_string(&config.orchestrator_dir.join(path)).ok())
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    let suggested = suggested_command(&cycle.status);

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "plan": {
                    "file": ".ai-orchestrator/plan.md",
                    "hash": plan_hash,
                    "steps_count": steps_count,
                    "current_step": cycle.step_id,
                },
                "git": {
                    "branch": branch,
                    "status": if clean { "clean" } else { "dirty" },
                    "last_commit": last_commit,
                },
                "patches": patches,
                "last_handoff": last_handoff.as_ref().map(|(handoff, path, modified)| json!({
                    "from": handoff.agent,
                    "to": handoff.target_agent,
                    "status": handoff.status,
                    "timestamp": chrono::DateTime::<chrono::Utc>::from(*modified).to_rfc3339(),
                    "file": path.display().to_string(),
                })),
                "last_audit": last_audit,
                "memory": {
                    "hash": memory_hash,
                    "last_sync": last_sync.last_sync,
                },
                "cycle": cycle,
                "next_command": suggested,
                "generated_at": now_string(),
            }))?
        );
        return Ok(());
    }

    if short {
        println!("status : {:?}", cycle.status);
        println!(
            "patch  : {}",
            cycle
                .patch_file
                .clone()
                .unwrap_or_else(|| "none".to_string())
        );
        println!(
            "audit  : {}",
            cycle
                .audit_approved
                .map(|v| if v { "approved" } else { "rejected" })
                .unwrap_or("pending")
        );
        println!("next   : {}", suggested);
        return Ok(());
    }

    println!("{}", "══════════════════════════════════════".bold());
    println!("{}", "AI ORCHESTRATOR — STATUS".bold());
    println!("{}", "══════════════════════════════════════".bold());
    println!("\nPLANO");
    println!("  Arquivo    : .ai-orchestrator/plan.md");
    println!("  Hash       : {plan_hash}");
    println!("  Steps      : {steps_count}");
    println!("  Step atual : {}", cycle.step_id);

    println!("\nGIT");
    println!("  Branch     : {branch}");
    println!("  Status     : {}", status_label(clean));
    println!("  Último commit: {last_commit}");

    println!("\nPATCHES");
    for patch in patches {
        println!(
            "  {} → {}",
            patch["file"].as_str().unwrap_or("unknown"),
            patch["status"].as_str().unwrap_or("unknown")
        );
    }

    println!("\nÚLTIMO HANDOFF");
    if let Some((handoff, _, modified)) = last_handoff {
        println!("  De         : {}", handoff.agent);
        println!("  Para       : {}", handoff.target_agent);
        println!("  Status     : {:?}", handoff.status);
        println!(
            "  Timestamp  : {}",
            chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339()
        );
    } else {
        println!("  Nenhum handoff encontrado.");
    }

    println!("\nÚLTIMA AUDITORIA");
    if let Some(value) = last_audit {
        let approved = value
            .pointer("/parsed/approved")
            .or_else(|| value.get("approved"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let score = value
            .pointer("/parsed/score")
            .or_else(|| value.get("score"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let problems = value
            .pointer("/parsed/problems")
            .or_else(|| value.get("problems"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        println!(
            "  Status     : {}",
            if approved {
                "APROVADO".green().to_string()
            } else {
                "REPROVADO".red().to_string()
            }
        );
        println!("  Score      : {score}");
        println!(
            "  Problemas  : {}",
            if problems.is_empty() {
                "Nenhum".to_string()
            } else {
                problems
                    .into_iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
                    .join("; ")
            }
        );
    } else {
        println!("  Nenhuma auditoria encontrada.");
    }

    println!("\nIA-MEMORY");
    println!("  Hash       : {memory_hash}");
    println!(
        "  Última sync: {}",
        last_sync
            .last_sync
            .map(|ts| ts.to_rfc3339())
            .unwrap_or_else(|| "never".to_string())
    );

    println!("\n{}", "══════════════════════════════════════".bold());
    println!("Próximo passo sugerido: {}", suggested.yellow());
    println!("{}", "══════════════════════════════════════".bold());
    Ok(())
}

fn count_steps(plan: &str) -> usize {
    let regex = Regex::new(r"(?m)^\d+\.\s+").expect("valid regex");
    regex.find_iter(plan).count()
}

fn collect_patches(
    config: &Config,
    cycle: &crate::schemas::CycleState,
) -> Result<Vec<serde_json::Value>> {
    let patches_dir = config.orchestrator_dir.join("patches");
    if !patches_dir.exists() {
        return Ok(vec![]);
    }

    let mut patches = Vec::new();
    for entry in std::fs::read_dir(&patches_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("diff") {
            continue;
        }

        let relative = path
            .strip_prefix(&config.orchestrator_dir)
            .unwrap_or(&path)
            .display()
            .to_string();
        let status = if cycle.patch_file.as_deref() == Some(relative.as_str()) {
            match cycle.status {
                CycleStatus::Approved => "audited:approved",
                CycleStatus::Rejected => "audited:rejected",
                CycleStatus::Applied => "applied",
                _ => "pending",
            }
        } else {
            "pending"
        };
        patches.push(json!({ "file": relative, "status": status }));
    }

    patches.sort_by(|a, b| a["file"].as_str().cmp(&b["file"].as_str()));
    Ok(patches)
}

fn suggested_command(status: &CycleStatus) -> &'static str {
    match status {
        CycleStatus::Initialized => "ai-orchestrator run",
        CycleStatus::DevDone | CycleStatus::AuditRequested => "ai-orchestrator audit",
        CycleStatus::Approved => "ai-orchestrator apply",
        CycleStatus::Rejected => "revise plan/contexto e execute ai-orchestrator run",
        CycleStatus::Applied => "git diff HEAD && git commit",
    }
}
