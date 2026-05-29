use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands;

#[derive(Debug, Parser)]
#[command(name = "ai-orchestrator", version, about = "AI multi-agent orchestrator")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
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
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { force, dry_run } => commands::init::execute(force, dry_run).await,
        Commands::Run { step, dry_run, manual, new_plan } => commands::run::execute(step, dry_run, manual, new_plan).await,
        Commands::Audit { patch, dry_run } => commands::audit::execute(patch, dry_run).await,
        Commands::Apply { patch, dry_run } => commands::apply::execute(patch, dry_run).await,
        Commands::Status { json, short, history, run_id } => commands::status::execute(json, short, history, run_id).await,
    }
}
