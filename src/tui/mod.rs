use anyhow::Result;
use colored::Colorize;
use inquire::{Select, Text};
use tokio::task;

pub mod dashboard;

use crate::cli::PlanCommands;

/// Ponto de entrada principal do modo TUI interativo (baseado em inquire).
/// Este módulo continuará existindo como "launcher" enquanto construímos o dashboard em Ratatui.
pub async fn start() -> Result<()> {
    println!("{}", "🚀 AI Orchestrator — Modo Interativo".bold().cyan());
    println!("Bem-vindo! Use as setas para navegar e Enter para selecionar.\n");

    loop {
        let options = vec![
            "🆕  Novo Plano",
            "🔄  Continuar Planejamento",
            "🚀  Executar Modo Dev",
            "📊  Abrir Dashboard (Ratatui)",
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
            "📊  Abrir Dashboard (Ratatui)" => {
                // Lança o dashboard em Ratatui (bloqueia até o usuário sair com 'q')
                let _ = crate::tui::dashboard::run_dashboard(None);
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

    crate::commands::plan::execute(PlanCommands::New { title: None }).await?;

    Ok(())
}

async fn handle_status() -> Result<()> {
    println!("{}", "\nExibindo status do projeto...".bold());

    crate::commands::status::execute(false, false, false, None).await
}

async fn handle_continue_planning() -> Result<()> {
    println!("{}", "\nContinuando planejamento...".bold());

    let plan_id: Option<String> = task::spawn_blocking(|| {
        Text::new("ID do plano (deixe vazio para selecionar interativamente):")
            .prompt()
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
    .await?;

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

    let max_turns: usize = task::spawn_blocking(|| {
        Text::new("Número máximo de turnos (padrão 10):")
            .with_default("10")
            .prompt()
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .unwrap_or(10)
    })
    .await?;

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

    let plan_id: String = task::spawn_blocking(|| {
        Text::new("ID do plano para executar:")
            .prompt()
    })
    .await??;

    crate::commands::run::execute(
        None,
        false,
        false,
        false,
        Some(plan_id),
    ).await
}