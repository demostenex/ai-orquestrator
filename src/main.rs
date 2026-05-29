#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ai_orchestrator::cli::run().await
}
