use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands;

#[derive(Debug, Parser)]
#[command(
    name = "ai-orchestrator",
    version,
    about = "AI multi-agent orchestrator"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Init {
        #[arg(long)]
        force: bool,
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
    Run {
        #[arg(long)]
        step: Option<String>,
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Modo manual: escreve prompts em arquivos e aguarda respostas dos CLIs externos
        #[arg(long)]
        manual: bool,
        /// Digitar novo plano direto no terminal antes de rodar
        #[arg(long = "new-plan")]
        new_plan: bool,
        /// Executa em Modo Dev orientado a tarefas de um plano aprovado (--plan <id>)
        #[arg(long = "plan", value_name = "PLAN_ID")]
        plan: Option<String>,
    },
    Audit {
        #[arg(long)]
        patch: Option<PathBuf>,
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
    Apply {
        #[arg(long)]
        patch: Option<PathBuf>,
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        short: bool,
        /// Mostra histórico de runs do projeto (via SQLite)
        #[arg(long)]
        history: bool,
        /// ID do run para ver trilha de auditoria detalhada
        #[arg(long = "run-id")]
        run_id: Option<String>,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
    Plan {
        #[command(subcommand)]
        command: PlanCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommands {
    #[command(about = "Lista todas as sessões PTY ativas")]
    List,
    #[command(about = "Encerra uma sessão PTY pelo seu PID")]
    Kill { pid: u32 },
}

#[derive(Debug, Subcommand)]
pub enum PlanCommands {
    #[command(about = "Cria um novo plano de alto nível")]
    New {
        #[arg(short, long, help = "Título descritivo do plano")]
        title: Option<String>,
    },
    #[command(about = "Finaliza o planejamento, bloqueia o todo list e exporta versão final")]
    Finalize {
        #[arg(short, long, help = "ID do plano a ser finalizado")]
        plan_id: String,
    },
    #[command(about = "Continua uma sessão de planejamento interativo")]
    Continue {
        #[arg(
            short,
            long,
            help = "ID do plano (se omitido, abre seletor interativo)"
        )]
        plan_id: Option<String>,
        #[arg(long, help = "CLI para o agente Arquiteto (obrigatório)")]
        cli1: String,
        #[arg(
            long,
            help = "CLI para o Revisor de planejamento (opcional; não implementa código)"
        )]
        cli2: Option<String>,
        #[arg(long, default_value_t = 10, help = "Número máximo de turnos")]
        max_turns: usize,
    },
    #[command(about = "Remove um plano e todo o seu conteúdo (tarefas, turnos, versões)")]
    Delete {
        #[arg(short, long, help = "ID do plano a ser removido")]
        plan_id: String,
        #[arg(long, help = "Pula a confirmação interativa")]
        yes: bool,
    },
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init { force, dry_run }) => commands::init::execute(force, dry_run).await,
        Some(Commands::Run {
            step,
            dry_run,
            manual,
            new_plan,
            plan,
        }) => commands::run::execute(step, dry_run, manual, new_plan, plan).await,
        Some(Commands::Audit { patch, dry_run }) => commands::audit::execute(patch, dry_run).await,
        Some(Commands::Apply { patch, dry_run }) => commands::apply::execute(patch, dry_run).await,
        Some(Commands::Status {
            json,
            short,
            history,
            run_id,
        }) => commands::status::execute(json, short, history, run_id).await,
        Some(Commands::Session { command }) => match command {
            SessionCommands::List => crate::commands::session::list().await,
            SessionCommands::Kill { pid } => crate::commands::session::kill(pid).await,
        },
        Some(Commands::Plan { command }) => commands::plan::execute(command).await,
        None => crate::tui::start().await,
    }
}
