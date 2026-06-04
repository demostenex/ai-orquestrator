use anyhow::{anyhow, Result};
use chrono::Utc;
use colored::Colorize;
use std::path::Path;
use uuid::Uuid;

use crate::cli::PlanCommands;
use crate::commands::{
    export_plan_to_markdown, interactive_gate, interactive_plan_gate, print_success, print_warning,
};
use crate::core::cli_runner::run_cli;
use crate::core::config::Config;
use crate::core::db::Db;
use crate::schemas::PlanTurn;

/// Ponto de entrada para o subcomando `plan`.
pub async fn execute(command: PlanCommands) -> Result<()> {
    match command {
        PlanCommands::New { title } => new_plan(title).await,
        PlanCommands::Finalize { plan_id } => finalize_plan(plan_id).await,
        PlanCommands::Continue {
            plan_id,
            cli1,
            cli2,
            max_turns,
        } => continue_plan(plan_id, cli1, cli2, max_turns).await,
        PlanCommands::Delete { plan_id, yes } => delete_plan_cmd(plan_id, yes).await,
    }
}

/// Remove um plano e todo o seu conteúdo. Pede confirmação a menos que `yes`.
async fn delete_plan_cmd(plan_id: String, yes: bool) -> Result<()> {
    let config = Config::load()?;
    let db = Db::open(
        &config.orchestrator_dir,
        &config.workspace_dir,
        &Uuid::new_v4().to_string(), // run_id temporário só para esta operação
        &config.step_id,
        "planning",
        "HEAD",
        "",
        "",
    )?;

    let title = match db.get_plan_title(&plan_id).await {
        Ok(t) => t,
        Err(_) => {
            return Err(anyhow!("Plano '{}' não encontrado.", plan_id));
        }
    };
    let tasks = db.get_plan_tasks(&plan_id).await.unwrap_or_default();

    if !yes {
        let prompt = format!(
            "Remover o plano \"{}\" (ID {}) e suas {} tarefa(s)? Esta ação é irreversível.",
            title,
            plan_id,
            tasks.len()
        );
        let confirmed = tokio::task::spawn_blocking(move || {
            inquire::Confirm::new(&prompt)
                .with_default(false)
                .prompt()
                .unwrap_or(false)
        })
        .await?;
        if !confirmed {
            println!("{}", "Cancelado.".yellow());
            return Ok(());
        }
    }

    let removed = db.delete_plan(&plan_id).await?;
    if removed == 0 {
        println!("{}", format!("Plano '{}' não encontrado.", plan_id).yellow());
    } else {
        println!(
            "{}",
            format!("✔ Plano \"{}\" removido.", title).green()
        );
    }
    Ok(())
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

    print_success("Plano criado com sucesso!");
    println!("  ID do plano : {}", plan_id.cyan());
    println!("  Título      : {}", plan_title);
    println!(
        "  Arquivo     : {}",
        config
            .orchestrator_dir
            .join("plan.md")
            .display()
            .to_string()
            .cyan()
    );
    println!();
    println!(
        "Use {} para começar a interagir com o plano.",
        "ai-orchestrator plan continue".yellow()
    );

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
    prompt.push_str(
        "- Mantenha o foco no planejamento de alto nível (arquitetura, tarefas, decisões).\n\n",
    );
    prompt
        .push_str("- Não escreva código de implementação, não gere diff e não aplique mudanças.\n");
    prompt.push_str(
        "- Se faltar contexto humano, peça esclarecimento ou proponha perguntas objetivas.\n\n",
    );

    prompt
}

/// Executa um turno completo de planejamento:
/// 1. Busca turnos anteriores
/// 2. Monta o prompt via build_planning_prompt
/// 3. Chama o CLI (via run_cli simples)
/// 4. Persiste o resultado via add_plan_turn
///
/// Retorna o PlanTurn recém-criado.
pub async fn run_planning_turn(
    db: &Db,
    plan_id: &str,
    agent: &str,
    role: &str,
    cli_name: &str,
    log: Option<crate::core::stream::LogTx>,
) -> Result<PlanTurn> {
    // 1. Buscar histórico de turnos
    let turns = db.get_plan_turns(plan_id).await?;

    // 2. Construir o prompt para este turno
    let prompt = build_planning_prompt(&turns, role);

    // 3. Executar o CLI (run_cli é síncrono → spawn_blocking)
    let content = tokio::task::spawn_blocking({
        let cli = cli_name.to_string();
        let p = prompt.clone();
        move || run_cli(&cli, &p, log.clone())
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

/// Executa o loop completo de planejamento interativo.
/// Alterna entre os agentes fornecidos até atingir `max_turns` ou o usuário abortar/finalizar no gate.
#[allow(clippy::too_many_arguments)]
pub async fn run_planning_loop(
    db: &Db,
    orchestrator_dir: &Path,
    plan_id: &str,
    agents: &[(&str, &str, &str)], // (agent, role, cli_name)
    max_turns: usize,
    initial_human_notes: Option<&str>,
    log: Option<crate::core::stream::LogTx>,
    gate_rx: Option<crate::core::stream::GateRx>,
) -> Result<()> {
    if agents.is_empty() {
        return Err(anyhow!("Lista de agentes não pode estar vazia"));
    }

    let log_line = |msg: String| {
        if let Some(ref tx) = log {
            let _ = tx.send(crate::core::stream::LogEvent::Line(msg.clone()));
        } else {
            println!("{}", msg);
        }
    };

    // Variante com origem estruturada: usada quando sabemos qual agente está
    // emitindo a linha (ancora o pane correto no TUI sem heurística de string).
    let log_agent_line = |msg: String, origin: crate::core::stream::LineOrigin| {
        if let Some(ref tx) = log {
            let _ = tx.send(crate::core::stream::LogEvent::AgentLine {
                text: msg.clone(),
                origin,
            });
        } else {
            println!("{}", msg);
        }
    };

    if let Some(notes) = initial_human_notes
        .map(str::trim)
        .filter(|notes| !notes.is_empty())
    {
        if db.get_plan_turns(plan_id).await?.is_empty() {
            db.add_plan_turn(
                plan_id,
                "human",
                "Briefing inicial do humano antes de qualquer agente",
                notes,
            )
            .await?;
            log_line("🧭 Briefing humano inicial registrado no plano.".to_string());
        }
    }

    let architect_idx = 0usize;
    let reviewer_idx = if agents.len() > 1 { Some(1usize) } else { None };
    let mut next_agent_idx = architect_idx;

    for turn_index in 0..max_turns {
        let (agent, role, cli_name) = agents[next_agent_idx];
        next_agent_idx = architect_idx;

        let turn_header = format!("══ TURNO {} | {} ({}) ══", turn_index + 1, agent, role);
        match crate::core::stream::LineOrigin::from_role(role) {
            Some(origin) => log_agent_line(turn_header, origin),
            None => log_line(turn_header),
        }

        let turn = run_planning_turn(db, plan_id, agent, role, cli_name, log.clone()).await?;

        // Gate: TUI nativo ou inquire (modo CLI)
        if let (Some(ref tx), Some(ref rx)) = (&log, &gate_rx) {
            let _ = tx.send(crate::core::stream::LogEvent::GateNeeded {
                content: turn.content.clone(),
                gate_type: "planning".to_string(),
            });
            match rx.recv() {
                Ok(crate::core::stream::GateDecision::Continue) => {
                    log_line(" ✅ Planejamento congelado pelo usuário.".to_string());
                    db.lock_plan_tasks(plan_id).await.ok();
                    let config = crate::core::config::Config::load()?;
                    crate::commands::export_plan_to_markdown(&config.orchestrator_dir, db, plan_id)
                        .await
                        .ok();
                    log_line(" ✅ Planejamento finalizado e exportado.".to_string());
                    return Ok(());
                }
                Ok(crate::core::stream::GateDecision::Review) => {
                    if let Some(idx) = reviewer_idx {
                        next_agent_idx = idx;
                        log_line(" 🔎 Enviando próximo turno ao Revisor...".to_string());
                    } else {
                        log_line(
                            " ⚠ Nenhum Revisor configurado. Voltando ao Arquiteto...".to_string(),
                        );
                    }
                }
                Ok(crate::core::stream::GateDecision::Enrich(notes)) => {
                    let hash = crate::core::compute_sha256(&turn.content);
                    db.add_plan_version(plan_id, &hash, Some(&notes)).await.ok();
                    db.add_plan_turn(
                        plan_id,
                        "human",
                        "Notas humanas adicionadas no gate de planejamento",
                        &notes,
                    )
                    .await
                    .ok();
                    log_line(" ✏️ Notas aplicadas. Voltando ao Arquiteto...".to_string());
                }
                Ok(crate::core::stream::GateDecision::Finalize) => {
                    log_line(" ✅ Finalizando planejamento...".to_string());
                    db.lock_plan_tasks(plan_id).await.ok();
                    let config = crate::core::config::Config::load()?;
                    crate::commands::export_plan_to_markdown(&config.orchestrator_dir, db, plan_id)
                        .await
                        .ok();
                    log_line(" ✅ Planejamento finalizado e exportado.".to_string());
                    return Ok(());
                }
                Ok(crate::core::stream::GateDecision::Abort) | Err(_) => {
                    log_line(" 🛑 Planejamento abortado.".to_string());
                    return Ok(());
                }
            }
        } else {
            match interactive_plan_gate(orchestrator_dir, db, plan_id, &turn.content).await {
                Ok(_) => println!(" {} Turno concluído. Avançando...\n", "✔".green()),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("planejamento_finalizado") {
                        println!(" {} Planejamento finalizado pelo usuário.\n", "✅".green());
                    } else {
                        println!(" {} Planejamento abortado pelo usuário.\n", "🛑".red());
                    }
                    return Ok(());
                }
            }
        }
    }

    log_line(format!(
        "Limite de {} turnos atingido. Planejamento encerrado.",
        max_turns
    ));
    Ok(())
}

/// Finaliza o planejamento:
/// - Pede confirmação explícita do usuário via gate
/// - Ativa write_locked em todas as tarefas do plano
/// - Exporta o todo list final em Markdown
pub async fn finalize_plan(plan_id: String) -> Result<()> {
    let config = Config::load()?;

    // Busca informações básicas do plano
    let db = Db::open(
        &config.orchestrator_dir,
        &config.workspace_dir,
        &Uuid::new_v4().to_string(), // run_id temporário só para leitura
        &config.step_id,
        "planning",
        "HEAD",
        "",
        "",
    )?;

    let title = db.get_plan_title(&plan_id).await?;
    let tasks = db.get_plan_tasks(&plan_id).await?;

    let summary = format!(
        "Plano: {}\nID: {}\n\nTotal de tarefas: {}\n\nDeseja realmente **finalizar** este planejamento?\n\nIsso irá bloquear permanentemente a edição do todo list (write_locked).",
        title,
        plan_id,
        tasks.len()
    );

    // Gate de confirmação humana (obrigatório)
    let _confirmation =
        interactive_gate("Finalizar Planejamento", &summary, "Humano → Sistema").await?;

    // Se o usuário abortou, o interactive_gate já retorna erro
    // Se chegou aqui, foi confirmado

    println!(
        "\n{} Confirmado. Finalizando planejamento...\n",
        "✔".green()
    );

    // Ativa o lock (só aqui, após confirmação explícita)
    db.lock_plan_tasks(&plan_id).await?;

    // Exporta o todo list final
    export_plan_to_markdown(&config.orchestrator_dir, &db, &plan_id).await?;

    print_success("Planejamento finalizado com sucesso!");
    println!("  ID do plano     : {}", plan_id.cyan());
    println!("  Título          : {}", title);
    println!("  Tarefas         : {}", tasks.len());
    println!(
        "  write_locked    : {}",
        "ativado em todas as tarefas".green()
    );
    println!(
        "  Arquivo gerado  : {}",
        config
            .orchestrator_dir
            .join("plan.md")
            .display()
            .to_string()
            .cyan()
    );
    println!();
    println!("O todo list agora está bloqueado para edição por agentes que não sejam 'dev'.");

    // Sync com ai-memory (best effort — não aborta se falhar)
    let summary = format!(
        "# Plano Finalizado\n\n**ID:** {}\n**Título:** {}\n**Tarefas:** {}\n**Data:** {}\n",
        plan_id,
        title,
        tasks.len(),
        chrono::Utc::now().to_rfc3339()
    );

    if let Err(e) = std::process::Command::new("ai-memory")
        .args([
            "write-page",
            "--path",
            &format!("decisions/plan_{}.md", plan_id),
            "--body",
            &summary,
        ])
        .output()
    {
        print_warning(&format!(
            "Não foi possível sincronizar com ai-memory: {}",
            e
        ));
    }

    Ok(())
}

async fn continue_plan(
    plan_id: Option<String>,
    cli1: String,
    cli2: Option<String>,
    max_turns: usize,
) -> Result<()> {
    let config = Config::load()?;

    // 1. Resolver plan_id (se None → seletor interativo)
    let plan_id = match plan_id {
        Some(id) => id,
        None => {
            let db = Db::open(
                &config.orchestrator_dir,
                &config.workspace_dir,
                &Uuid::new_v4().to_string(),
                &config.step_id,
                "planning",
                "HEAD",
                "",
                "",
            )?;

            let plans = db.list_plans().await?;
            if plans.is_empty() {
                return Err(anyhow!(
                    "Nenhum plano encontrado. Use 'ai-orchestrator plan new' primeiro."
                ));
            }

            let options: Vec<String> = plans
                .iter()
                .map(|p| format!("{} — {}", p.id, p.title))
                .collect();

            let selection = tokio::task::spawn_blocking(move || {
                inquire::Select::new("Selecione o plano para continuar:", options).prompt()
            })
            .await??;

            selection
                .split(" — ")
                .next()
                .ok_or_else(|| anyhow!("Seleção inválida"))?
                .to_string()
        }
    };

    // 2. Monta a lista de agentes
    let agents: Vec<(&str, &str, &str)> = if let Some(cli_reviewer) = &cli2 {
        vec![
            ("architect", "Arquiteto", cli1.as_str()),
            ("reviewer", "Revisor de Planejamento", cli_reviewer.as_str()),
        ]
    } else {
        vec![("architect", "Arquiteto", cli1.as_str())]
    };

    // 3. Abre o DB para o loop
    let db = Db::open(
        &config.orchestrator_dir,
        &config.workspace_dir,
        &Uuid::new_v4().to_string(),
        &config.step_id,
        "planning",
        "HEAD",
        "",
        "",
    )?;

    // 4. Executa o loop de planejamento
    run_planning_loop(
        &db,
        &config.orchestrator_dir,
        &plan_id,
        &agents,
        max_turns,
        None,
        None,
        None,
    )
    .await?;

    Ok(())
}
