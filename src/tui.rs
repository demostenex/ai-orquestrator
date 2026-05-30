use anyhow::Result;
use colored::Colorize;
use inquire::Select;

use crate::cli::PlanCommands;

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
                println!("(Funcionalidade em desenvolvimento na Fase 2)\n");
            }
            "🚀  Executar Modo Dev" => {
                println!("(Funcionalidade em desenvolvimento na Fase 2)\n");
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