use anyhow::Result;
use colored::Colorize;
use inquire::{Select, Text};
use tokio::task;

use crate::cli::{Commands, PlanCommands};

/// Ponto de entrada principal do modo TUI interativo.
pub async fn start() -> Result<()> {
    println!("{}", "🚀 AI Orchestrator — Modo Interativo".bold().cyan());
    println!("Bem-vindo! Use as setas para navegar e Enter para selecionar.\n");

    loop {
        let options = vec![
            "🆕  Novo Plano",
            "🔄  Continuar Planejamento",
            "🚀  Executar Modo Dev",
            "📊  Status do Projeto",
            "🚪  Sair",
        ];

        let selection = Select::new("O que deseja fazer?", options)
            .with_help_message("Use ↑↓ e Enter")
            .prompt()?;

        match selection {
            "🆕  Novo Plano" => {
                handle_new_plan().await?;
            }
            "🔄  Continuar Planejamento" => {
                handle_continue_planning().await?;
            }
            "🚀  Executar Modo Dev" => {
                handle_execute_dev_mode().await?;
            }
            "📊  Status do Projeto" => {
                handle_status().await?;
            }
            "🚪  Sair" => {
                println!("Até logo!\n");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn handle_new_plan() -> Result<()> {
    println!("{}", "\nIniciando criação de novo plano...".bold());

    // Reutiliza diretamente a lógica de criação de plano
    crate::commands::plan::execute(PlanCommands::New { title: None }).await?;

    Ok(())
}

async fn handle_status() -> Result<()> {
    println!("{}", "\nExibindo status do projeto...".bold());

    // Reutiliza a lógica de status (sem json, sem history, sem run_id)
    crate::commands::status::execute(false, false, false, None).await
}

async fn handle_continue_planning() -> Result<()> {
    println!("{}", "\nContinuando planejamento...".bold());

    // 1. Perguntar pelo plan_id (pode ser omitido para seletor interativo)
    let plan_id: Option<String> = task::spawn_blocking(|| {
        Text::new("ID do plano (deixe vazio para selecionar interativamente):")
            .prompt()
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
    .await?;

    // 2. Perguntar pelos CLIs (tratamento exigido na Fase 3)
    let cli1: String = task::spawn_blocking(|| {
        Text::new("CLI para o agente Arquiteto (ex: claude, gemini):")
            .prompt()
    })
    .await??;

    let cli2: Option<String> = task::spawn_blocking(|| {
        Text::new("CLI para o agente Dev (opcional, pressione Enter para usar o mesmo):")
            .prompt()
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
    .await?;

    // 3. Perguntar max_turns
    let max_turns: usize = task::spawn_blocking(|| {
        Text::new("Número máximo de turnos (padrão 10):")
            .with_default("10")
            .prompt()
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .unwrap_or(10)
    })
    .await?;

    // 4. Chamar a lógica existente
    let continue_cmd = PlanCommands::Continue {
        plan_id,
        cli1,
        cli2,
        max_turns,
    };

    crate::commands::plan::execute(continue_cmd).await
}

async fn handle_execute_dev_mode() -> Result<()> {
    println!("{}", "\nExecutando Modo Dev...".bold());

    // Perguntar pelo ID do plano
    let plan_id: String = task::spawn_blocking(|| {
        Text::new("ID do plano para executar:")
            .prompt()
    })
    .await??;

    // Chamar o execute do run passando o plano
    crate::commands::run::execute(
        None,           // step
        false,          // dry_run
        false,          // manual
        false,          // new_plan
        Some(plan_id),  // plan
    ).await
}