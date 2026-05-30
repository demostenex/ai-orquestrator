use anyhow::Result;
use inquire::Text;
use tokio::task;

pub mod dashboard;
pub mod home;
pub mod plan_selector;

use crate::cli::PlanCommands;
use crate::core::db::Db;
use home::HomeAction;

/// Ponto de entrada principal — abre a Home Screen Ratatui e despacha ações.
/// Para inputs de texto, sai temporariamente do modo raw, usa inquire e retorna.
pub async fn start() -> Result<()> {
    let config = crate::core::config::Config::load()?;
    let orchestrator_dir = config.orchestrator_dir.clone();

    loop {
        let summary = Db::get_home_summary(orchestrator_dir.clone())
            .await
            .unwrap_or_default();

        let action = home::run(summary)?;

        match action {
            HomeAction::NewPlan => handle_new_plan().await?,
            HomeAction::ContinuePlanning => {
                let db = Db::open_readonly(&orchestrator_dir).await?;
                let plans = db.list_plans().await?;
                let plan_id = plan_selector::run(plans, "Selecionar plano para continuar o planejamento")?;
                handle_continue_planning(plan_id).await?;
            }
            HomeAction::ExecuteDev => handle_execute_dev_mode().await?,
            HomeAction::OpenDashboard => {
                let db = Db::open_readonly(&orchestrator_dir).await?;
                let plans = db.list_plans().await?;
                if let Some(pid) = plan_selector::run(plans, "Selecionar plano para abrir no Dashboard")? {
                    dashboard::run_dashboard(orchestrator_dir.clone(), pid)?;
                }
            }
            HomeAction::Quit => {
                println!("Até logo!");
                break;
            }
        }
    }

    Ok(())
}

// ── Handlers (fora do modo raw — usam inquire normalmente) ────────────────────

async fn handle_new_plan() -> Result<()> {
    crate::commands::plan::execute(PlanCommands::New { title: None }).await
}

async fn handle_continue_planning(plan_id: Option<String>) -> Result<()> {
    if plan_id.is_none() {
        return Ok(()); // cancelado no seletor
    }

    let cli1 = task::spawn_blocking(|| {
        Text::new("CLI para o Arquiteto (ex: claude, gemini):").prompt()
    })
    .await??;

    let cli2 = task::spawn_blocking(|| {
        Text::new("CLI para o Dev (vazio = mesmo que Arquiteto):")
            .prompt()
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
    .await?;

    let max_turns = task::spawn_blocking(|| {
        Text::new("Número máximo de turnos:")
            .with_default("10")
            .prompt()
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .unwrap_or(10usize)
    })
    .await?;

    crate::commands::plan::execute(PlanCommands::Continue {
        plan_id,
        cli1,
        cli2,
        max_turns,
    })
    .await
}

async fn handle_execute_dev_mode() -> Result<()> {
    let plan_id = task::spawn_blocking(|| {
        Text::new("ID do plano para executar:").prompt()
    })
    .await??;

    crate::commands::run::execute(None, false, false, false, Some(plan_id)).await
}
