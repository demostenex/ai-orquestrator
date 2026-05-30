use anyhow::{anyhow, Result};
use chrono::Utc;
use colored::Colorize;
use uuid::Uuid;

use crate::cli::PlanCommands;
use crate::commands::{export_plan_to_markdown, print_success};
use crate::core::cli_runner::run_cli;
use crate::core::config::Config;
use crate::core::db::Db;
use crate::schemas::PlanTurn;

/// Ponto de entrada para o subcomando `plan`.
pub async fn execute(command: PlanCommands) -> Result<()> {
    match command {
        PlanCommands::New { title } => new_plan(title).await,
    }
}

async fn new_plan(title: Option<String>) -> Result<()> {
    let config = Config::load()?;

    // Título: prompt interativo se não fornecido
    let plan_title = match title {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            // Usa inquire (já dependência do projeto)
            let input = tokio::task::spawn_blocking(|| {
                inquire::Text::new("Título do novo plano:")
                    .with_help_message("Ex: Refatoração do sistema de autenticação")
                    .prompt()
            })
            .await??;

            if input.trim().is_empty() {
                return Err(anyhow!("Título não pode ser vazio"));
            }
            input.trim().to_string()
        }
    };

    let plan_id = Uuid::new_v4().to_string();

    // Para V1 do planejamento, criamos um run dedicado ao planejamento
    // (pode evoluir para run_id NULL no futuro)
    let planning_run_id = Uuid::new_v4().to_string();
    let step_id = config.step_id.clone();

    // Abre o banco (cria o run de planejamento)
    let db = Db::open(
        &config.orchestrator_dir,
        &config.workspace_dir,
        &planning_run_id,
        &step_id,
        "planning",
        "HEAD", // base_commit placeholder
        "",     // plan_hash (será atualizado depois)
        "",     // memory_hash
    )?;

    // Cria o plano no SQLite
    db.create_plan(&plan_id, &plan_title).await?;

    // Exporta o plan.md inicial (vazio por enquanto)
    export_plan_to_markdown(&config.orchestrator_dir, &db, &plan_id).await?;

    print_success(&format!("Plano criado com sucesso!"));
    println!("  ID do plano : {}", plan_id.cyan());
    println!("  Título      : {}", plan_title);
    println!("  Arquivo     : {}", config.orchestrator_dir.join("plan.md").display().to_string().cyan());
    println!();
    println!("Use {} para começar a interagir com o plano.", "ai-orchestrator plan continue".yellow());

    Ok(())
}

/// Função pura que monta o prompt para o próximo turno de planejamento.
/// Recebe o histórico completo de turnos e o papel da IA atual.
pub fn build_planning_prompt(turns: &[crate::schemas::PlanTurn], role: &str) -> String {
    let mut prompt = String::new();

    prompt.push_str(&format!(
        "Você é a IA **{}** participando de um processo de planejamento colaborativo.\n\n",
        role
    ));

    if turns.is_empty() {
        prompt.push_str("Este é o primeiro turno do planejamento.\n\n");
    } else {
        prompt.push_str("Abaixo está o histórico completo dos turnos anteriores:\n\n");

        for turn in turns {
            prompt.push_str(&format!(
                "--- Turno {} | Agente: {} | {}\n",
                turn.sequence, turn.agent, turn.timestamp
            ));
            prompt.push_str(&format!("Prompt enviado:\n{}\n\n", turn.prompt.trim()));
            prompt.push_str(&format!("Resposta recebida:\n{}\n\n", turn.content.trim()));
        }
    }

    prompt.push_str("Sua tarefa agora:\n");
    prompt.push_str("- Analise cuidadosamente o histórico acima.\n");
    prompt.push_str("- Contribua de forma estruturada e útil de acordo com seu papel.\n");
    prompt.push_str("- Mantenha o foco no planejamento de alto nível (arquitetura, tarefas, decisões).\n\n");

    prompt
}

/// Executa um turno completo de planejamento:
/// 1. Busca turnos anteriores
/// 2. Monta o prompt via build_planning_prompt
/// 3. Chama o CLI (via run_cli simples)
/// 4. Persiste o resultado via add_plan_turn
/// Retorna o PlanTurn recém-criado.
pub async fn run_planning_turn(
    db: &Db,
    plan_id: &str,
    agent: &str,
    role: &str,
    cli_name: &str,
) -> Result<PlanTurn> {
    // 1. Buscar histórico de turnos
    let turns = db.get_plan_turns(plan_id).await?;

    // 2. Construir o prompt para este turno
    let prompt = build_planning_prompt(&turns, role);

    // 3. Executar o CLI (run_cli é síncrono → spawn_blocking)
    let content = tokio::task::spawn_blocking({
        let cli = cli_name.to_string();
        let p = prompt.clone();
        move || run_cli(&cli, &p)
    })
    .await??;

    // 4. Persistir o turno no banco
    db.add_plan_turn(plan_id, agent, &prompt, &content).await?;

    // 5. Retornar o turno criado
    let now = Utc::now().to_rfc3339();
    let sequence = (turns.len() as i64) + 1;

    Ok(PlanTurn {
        sequence,
        agent: agent.to_string(),
        prompt,
        content,
        timestamp: now,
    })
}
