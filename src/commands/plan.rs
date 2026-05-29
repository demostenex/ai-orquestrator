use anyhow::Result;

use crate::cli::PlanArgs;

/// Stub mínimo do comando `plan` (Passo 4.1 - esqueleto apenas).
pub async fn execute(_args: PlanArgs) -> Result<()> {
    println!("Comando 'plan' ainda não implementado (Passo 4.1).");
    Ok(())
}
