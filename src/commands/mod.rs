pub mod apply;
pub mod audit;
pub mod init;
pub mod plan;
pub mod run;
pub mod session;
pub mod status;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use colored::Colorize;
use serde::Serialize;

use crate::core::db::Db;
use crate::schemas::{CycleDecision, CycleResult, CycleState, Task};

pub(crate) fn read_to_string(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read file {}", path.display()))
}

pub(crate) fn write_string(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content).with_context(|| format!("failed to write file {}", path.display()))
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let content = serde_json::to_string_pretty(value)?;
    write_string(path, &content)
}

pub(crate) fn save_cycle(orchestrator_dir: &Path, cycle: &CycleState) -> Result<()> {
    let path = orchestrator_dir.join("current-cycle.json");
    write_json(&path, cycle)
}

pub(crate) fn load_cycle(orchestrator_dir: &Path) -> Result<CycleState> {
    let path = orchestrator_dir.join("current-cycle.json");
    let raw = read_to_string(&path)?;
    let cycle: CycleState = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse cycle state {}", path.display()))?;
    cycle.validate()?;
    Ok(cycle)
}

pub(crate) fn status_label(ok: bool) -> String {
    if ok {
        "clean".green().to_string()
    } else {
        "dirty".red().to_string()
    }
}

pub(crate) fn print_step(message: &str) {
    println!("{} {}", "→".blue(), message);
}

pub(crate) fn print_success(message: &str) {
    println!("{} {}", "✔".green(), message);
}

pub(crate) fn print_warning(message: &str) {
    println!("{} {}", "⚠".yellow(), message);
}

pub(crate) fn now_string() -> String {
    Utc::now().to_rfc3339()
}

// CA-MD1, CA-MD2, CA-MD4: Exportador Markdown
pub async fn export_plan_to_markdown(
    orchestrator_dir: &Path,
    db: &Db,
    plan_id: &str,
) -> Result<()> {
    let title = db.get_plan_title(plan_id).await?;
    let tasks = db.get_plan_tasks(plan_id).await?;

    let mut content = String::new();
    // CA-MD4: Cabeçalho obrigatório
    content.push_str("<!-- ⚠️ ESTE ARQUIVO É GERADO AUTOMATICAMENTE PELO AI-ORQUESTRATOR. NÃO EDITE DIRETAMENTE. -->\n\n");
    content.push_str(&format!("# {}\n\n", title));
    content.push_str("## Todo List\n\n");

    if tasks.is_empty() {
        content.push_str("  (nenhuma tarefa definida)\n");
    } else {
        for task in tasks {
            let mark = match task.status.as_str() {
                "completed" => "[x]",
                "pending" => "[ ]",
                "blocked" => "[!]",
                _ => "[ ]",
            };
            let assigned = task
                .assigned_to
                .as_ref()
                .map(|a| format!(" (@{})", a))
                .unwrap_or_default();
            content.push_str(&format!("- {} {}{}\n", mark, task.description, assigned));
        }
    }

    content.push_str("\n---\n*Nota: Este arquivo é uma exportação do SQLite. O banco de dados é a fonte da verdade.*\n");

    // Gerar e salvar hash antes de escrever o arquivo (CA-MD2)
    let hash = crate::core::compute_sha256(&content);
    db.add_plan_version(plan_id, &hash, Some("Auto-export"))
        .await?;

    let path = orchestrator_dir.join("plan.md");
    write_string(&path, &content)?;
    Ok(())
}

// CA-MD3: Detector de Conflito
pub async fn verify_plan_integrity(
    orchestrator_dir: &Path,
    db: &Db,
    plan_id: &str,
) -> Result<bool> {
    let path = orchestrator_dir.join("plan.md");
    if !path.exists() {
        return Ok(true); // Se não existe, não há conflito de edição
    }

    let file_content = read_to_string(&path)?;
    let file_hash = crate::core::compute_sha256(&file_content);

    let db_hash = db.get_latest_plan_hash(plan_id).await?;

    match db_hash {
        Some(expected) => Ok(file_hash == expected),
        None => Ok(false), // Temos arquivo mas nenhuma versão registrada no DB = Conflito
    }
}

/// Exibe o conteúdo com limite de linhas e oferece continuar ou truncar.
fn print_content_preview(content: &str, max_lines: usize) {
    let lines: Vec<&str> = content.lines().collect();
    let shown = lines.len().min(max_lines);

    // B1: Detecção estrita de Unified Diff para evitar falsos positivos com Markdown
    let is_diff = (content.contains("\n--- ") || content.starts_with("--- "))
        && (content.contains("\n+++ ") || content.starts_with("+++ "))
        && content.contains("\n@@");

    let mut in_diff_block = is_diff
        && (content.starts_with("--- ")
            || content.starts_with("+++ ")
            || content.starts_with("@@"));

    for line in &lines[..shown] {
        if is_diff
            && (line.starts_with("--- ") || line.starts_with("+++ ") || *line == "--- DIFF ---")
        {
            in_diff_block = true;
        }

        if in_diff_block {
            if line.starts_with('+') && !line.starts_with("+++") {
                println!("{}", line.green());
            } else if line.starts_with('-') && !line.starts_with("---") {
                println!("{}", line.red());
            } else if line.starts_with("@@") {
                println!("{}", line.cyan());
            } else if line.starts_with("+++") || line.starts_with("---") {
                println!("{}", line.bold());
            } else {
                println!("{}", line.dimmed());
            }
        } else {
            println!("{}", line.dimmed());
        }
    }
    if lines.len() > max_lines {
        println!(
            "{} ({} linhas truncadas — arquivo completo no mailbox)",
            "...".dimmed(),
            lines.len() - max_lines
        );
    }
}

/// Portão interativo: exibe conteúdo, permite adicionar informações e confirma antes de passar.
/// Retorna o conteúdo original + adições do usuário (se houver).
pub(crate) async fn interactive_gate(
    label: &str,
    content: &str,
    direction: &str, // ex: "Dev → Auditora", "Orquestrador → Dev"
) -> Result<String> {
    let sep = "══════════════════════════════════════════════════════════".bold();
    println!("\n{sep}");
    println!("{}", format!(" 📋 PORTÃO — {label}").bold().cyan());
    println!("{}", format!(" 🚀 Fluxo: {direction}").dimmed());
    println!("{sep}");
    print_content_preview(content, 60);
    println!("{sep}");

    let options = vec![
        "Seguir (Aprovar) ✅",
        "Enriquecer (Adicionar notas) ✏️",
        "Abortar (Cancelar) ❌",
    ];

    let selection = tokio::task::spawn_blocking(move || {
        inquire::Select::new("Selecione uma ação:", options)
            .with_help_message("Use as setas para navegar e Enter para confirmar")
            .prompt()
    })
    .await??;

    match selection {
        "Seguir (Aprovar) ✅" => {
            println!(" {} Prosseguindo...\n", "✔".green());
            Ok(content.to_string())
        }
        "Enriquecer (Adicionar notas) ✏️" => {
            let notes = tokio::task::spawn_blocking(|| {
                inquire::Text::new("Digite suas notas (ou deixe vazio para cancelar):")
                    .with_help_message("Estas notas serão injetadas no contexto da próxima IA")
                    .prompt()
            })
            .await??;

            if notes.trim().is_empty() {
                println!(
                    " {} Nenhuma nota adicionada. Prosseguindo...\n",
                    "⚠".yellow()
                );
                Ok(content.to_string())
            } else {
                println!(" {} Notas adicionadas ao contexto.", "✔".green());
                Ok(format!("{content}\n\n[NOTAS DO HUMANO]\n{notes}"))
            }
        }
        _ => {
            println!(" {} Operação abortada pelo usuário.", "✘".red());
            anyhow::bail!("Handoff cancelado")
        }
    }
}

/// Portão interativo específico para o contexto de um Plano.
/// Implementa o fluxo de "Enriquecer Plano" (Passo 2.4), capturando notas
/// semânticas do usuário que serão enviadas à IA para que ELA atualize o SSOT.
pub async fn interactive_plan_gate(
    _orchestrator_dir: &Path,
    db: &crate::core::db::Db,
    plan_id: &str,
    plan_content: &str,
) -> Result<String> {
    let sep = "══════════════════════════════════════════════════════════".bold();
    println!("\n{sep}");
    println!("{}", " 📋 PORTÃO — Revisão do Plano".bold().cyan());
    println!("{sep}");
    print_content_preview(plan_content, 60);
    println!("{sep}");

    let options = vec![
        "Seguir (Aprovar) ✅",
        "Enriquecer (Adicionar instruções/notas) ✏️",
        "Finalizar Planejamento ✅",
        "Abortar (Cancelar) ❌",
    ];

    let selection = tokio::task::spawn_blocking(move || {
        inquire::Select::new("Selecione uma ação:", options).prompt()
    })
    .await??;

    match selection {
        "Seguir (Aprovar) ✅" => Ok(plan_content.to_string()),
        "Enriquecer (Adicionar instruções/notas) ✏️" => {
            let notes = tokio::task::spawn_blocking(move || {
                inquire::Editor::new("Digite as instruções para a IA atualizar o plano:")
                    .with_help_message("O arquivo será aberto no seu editor padrão ($EDITOR). Feche o editor para salvar.")
                    .prompt()
            }).await??;

            if notes.trim().is_empty() {
                println!(
                    " {} Nenhuma instrução adicionada. Prosseguindo...\n",
                    "⚠".yellow()
                );
                Ok(plan_content.to_string())
            } else {
                db.add_plan_turn(
                    plan_id,
                    "human",
                    "Enriquecimento manual via portão do plano",
                    &notes,
                )
                .await?;
                println!(
                    " {} Instruções adicionadas ao contexto da próxima IA.",
                    "✔".green()
                );
                Ok(format!("{plan_content}\n\n[NOTAS DO HUMANO]\n{notes}"))
            }
        }
        "Finalizar Planejamento ✅" => anyhow::bail!("planejamento_finalizado"),
        "Abortar (Cancelar) ❌" => anyhow::bail!("planejamento_abortado"),
        _ => anyhow::bail!("Operação abortada no portão do plano."),
    }
}

/// Aguarda um arquivo aparecer no sistema de arquivos (modo manual).
/// Faz polling a cada 2 segundos com timeout de `max_wait_secs`.
pub(crate) async fn wait_for_file(path: &Path, max_wait_secs: u64) -> Result<()> {
    let mut elapsed = 0u64;
    while elapsed < max_wait_secs {
        if path.exists() && fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        elapsed += 2;
        if elapsed.is_multiple_of(10) {
            println!(
                "{} aguardando {} ({elapsed}s)...",
                "⏳".yellow(),
                path.file_name().unwrap_or_default().to_string_lossy()
            );
        }
    }
    anyhow::bail!(
        "timeout: arquivo não criado em {max_wait_secs}s: {}",
        path.display()
    )
}

/// Exibe instruções para o usuário executar o CLI externo e salvar a resposta.
pub(crate) fn print_manual_instructions(
    agent_label: &str,
    prompt_file: &Path,
    response_file: &Path,
) {
    let separator = "══════════════════════════════════════".bold();
    println!("\n{separator}");
    println!("{}", format!("MODO MANUAL — {agent_label}").bold().yellow());
    println!("{separator}");
    println!(
        "Prompt saved in:\n  {}\n",
        prompt_file.display().to_string().cyan()
    );
    println!("Execute in another terminal:");
    println!(
        "  {} {}\n",
        "cat".dimmed(),
        prompt_file.display().to_string().cyan()
    );
    println!("  (or direct pipe to your CLI: gemini, claude, copilot)\n");
    println!("Save JSON response in:");
    println!("  {}\n", response_file.display().to_string().green());
    println!(
        "{}",
        "The orchestrator is waiting for file creation...".dimmed()
    );
    println!("{separator}\n");
}

pub(crate) fn latest_patch_file(orchestrator_dir: &Path) -> Result<Option<PathBuf>> {
    let patches_dir = orchestrator_dir.join("patches");
    if !patches_dir.exists() {
        return Ok(None);
    }

    let mut latest: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in fs::read_dir(&patches_dir)
        .with_context(|| format!("failed to read directory {}", patches_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("diff") {
            continue;
        }
        let modified = entry
            .metadata()
            .with_context(|| format!("failed to read metadata for {}", path.display()))?
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let should_replace = latest
            .as_ref()
            .map(|(_, current)| modified > *current)
            .unwrap_or(true);
        if should_replace {
            latest = Some((path, modified));
        }
    }

    Ok(latest.map(|entry| entry.0))
}

/// Portão interativo entre tarefas no Modo Plano (--plan).
///
/// Deve ser chamado após cada execução de `run_dev_cycle`.
/// Retorna a decisão do humano, que deve ter precedência sobre o `CycleDecision` técnico.
pub async fn interactive_inter_task_gate(
    current_task: Option<&Task>,
    last_cycle: &CycleResult,
) -> Result<CycleDecision> {
    let sep = "══════════════════════════════════════════════════════════".bold();

    println!("\n{sep}");
    println!("{}", " 📋 PORTÃO INTER-TAREFA".bold().cyan());
    println!("{}", " (Decisão humana entre tarefas do plano)".dimmed());
    println!("{sep}");

    // Contexto da tarefa atual
    if let Some(task) = current_task {
        println!("Tarefa atual: {}", task.description.trim().cyan());
        println!("ID: {}", task.id);
    }

    // Resultado do último ciclo
    let status = if last_cycle.approved {
        "APROVADO".green().bold()
    } else {
        "REPROVADO".red().bold()
    };

    println!(
        "Resultado do ciclo: {} (aplicado: {})",
        status,
        if last_cycle.applied { "sim" } else { "não" }
    );

    if !last_cycle.summary.is_empty() {
        println!("Resumo: {}", last_cycle.summary);
    }

    if last_cycle.approved {
        println!("Status: {}", "Ciclo aprovado pela Auditora".green());
    } else {
        println!("Status: {}", "Ciclo reprovado pela Auditora".red());
    }

    println!("{sep}");

    let default_avancar = last_cycle.approved;

    let options = vec![
        "Avançar para próxima tarefa ✅",
        "Repetir esta tarefa 🔄",
        "Enriquecer e Repetir ✏️",
        "Abortar plano ❌",
    ];

    let default_index = if default_avancar { 0 } else { 1 };

    let selection = tokio::task::spawn_blocking(move || {
        inquire::Select::new("O que deseja fazer?", options)
            .with_starting_cursor(default_index)
            .with_help_message("Use as setas ↑↓ e Enter para confirmar")
            .prompt()
    })
    .await??;

    match selection {
        "Avançar para próxima tarefa ✅" => {
            println!(" {} Avançando para a próxima tarefa...\n", "✔".green());
            Ok(CycleDecision::Proceed)
        }

        "Repetir esta tarefa 🔄" => {
            let notes = tokio::task::spawn_blocking(|| {
                inquire::Text::new("Notas para repetir a tarefa (opcional):")
                    .with_help_message(
                        "Estas notas serão injetadas no próximo prompt da mesma tarefa",
                    )
                    .prompt()
            })
            .await??;

            let notes = if notes.trim().is_empty() {
                None
            } else {
                Some(notes)
            };

            println!(" {} Repetindo a mesma tarefa...\n", "🔄".yellow());
            Ok(CycleDecision::RepeatCurrentTask { user_notes: notes })
        }

        "Enriquecer e Repetir ✏️" => {
            let notes = tokio::task::spawn_blocking(|| {
                inquire::Editor::new("Instruções/notas para enriquecer e repetir:")
                    .with_help_message("Use seu editor padrão. Feche para confirmar.")
                    .prompt()
            })
            .await??;

            if notes.trim().is_empty() {
                println!(
                    " {} Nenhuma nota adicionada. Prosseguindo com repetição simples...\n",
                    "⚠".yellow()
                );
                Ok(CycleDecision::RepeatCurrentTask { user_notes: None })
            } else {
                println!(" {} Repetindo com enriquecimento...\n", "✏️".cyan());
                Ok(CycleDecision::EnrichAndRepeat { user_notes: notes })
            }
        }

        "Abortar plano ❌" => {
            println!(" {} Abortando execução do plano.\n", "✘".red());
            Ok(CycleDecision::Abort)
        }

        _ => Ok(CycleDecision::Abort),
    }
}
