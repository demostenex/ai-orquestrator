use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Result};
use colored::Colorize;

/// CLIs conhecidos e como passam o prompt (via stdin)
const KNOWN_CLIS: &[(&str, &str)] = &[
    ("gemini",   "Google Gemini CLI"),
    ("claude",   "Anthropic Claude CLI"),
    ("llm",      "Simon Willison's LLM CLI"),
    ("aichat",   "aichat CLI"),
    ("tgpt",     "tgpt CLI"),
    ("sgpt",     "ShellGPT CLI"),
    ("copilot",  "GitHub Copilot CLI"),
];

/// Detecta quais CLIs conhecidos estão instalados no PATH.
pub fn detect_available_clis() -> Vec<(&'static str, &'static str)> {
    KNOWN_CLIS
        .iter()
        .filter(|(cmd, _)| is_cli_available(cmd))
        .copied()
        .collect()
}

/// Verifica se um CLI está disponível no PATH.
pub fn is_cli_available(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Menu interativo para o usuário escolher um CLI disponível.
/// Retorna o comando escolhido ou None se não houver nenhum disponível.
pub fn select_cli(role: &str) -> Result<Option<String>> {
    let available = detect_available_clis();

    if available.is_empty() {
        println!("{}", format!("⚠ Nenhum CLI conhecido encontrado no PATH para o papel de {role}.").yellow());
        println!("  CLIs suportados: {}", KNOWN_CLIS.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(", "));
        println!("  Você pode digitar o comando manualmente ou pressionar Enter para usar modo manual.");
        print!("  Comando CLI para {role} (ou Enter para manual): ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        // Valida que o comando digitado existe
        if !is_cli_available(trimmed) {
            return Err(anyhow!("CLI '{trimmed}' não encontrado no PATH."));
        }
        return Ok(Some(trimmed.to_string()));
    }

    println!("\n{}", format!("Escolha o CLI para IA {role}:").bold());
    for (i, (cmd, desc)) in available.iter().enumerate() {
        println!("  [{}] {} — {}", i + 1, cmd.cyan(), desc);
    }
    println!("  [{}] Outro (digitar comando)", available.len() + 1);
    println!("  [{}] Modo manual (você cola o prompt)", available.len() + 2);
    print!("  Opção: ");
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input.trim().parse().unwrap_or(0);

    if choice >= 1 && choice <= available.len() {
        let cmd = available[choice - 1].0.to_string();
        println!("  {} selecionado: {}", role, cmd.green());
        return Ok(Some(cmd));
    }

    if choice == available.len() + 1 {
        print!("  Comando CLI para {role}: ");
        std::io::stdout().flush()?;
        let mut custom = String::new();
        std::io::stdin().read_line(&mut custom)?;
        let trimmed = custom.trim().to_string();
        if !is_cli_available(&trimmed) {
            return Err(anyhow!("CLI '{trimmed}' não encontrado no PATH."));
        }
        println!("  {} selecionado: {}", role, trimmed.green());
        return Ok(Some(trimmed));
    }

    // Modo manual
    Ok(None)
}

/// Envia o prompt para o CLI via stdin e exibe a resposta em streaming (linha a linha),
/// acumulando tudo para retornar ao final.
pub fn run_cli(cli_cmd: &str, prompt: &str) -> Result<String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cli_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow!("falha ao executar CLI '{cli_cmd}': {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }

    let stdout = child.stdout.take()
        .ok_or_else(|| anyhow!("stdout não disponível para '{cli_cmd}'"))?;

    let reader = BufReader::new(stdout);
    let mut full_output = String::new();

    for line in reader.lines() {
        let line = line?;
        println!("  {}", line);
        full_output.push_str(&line);
        full_output.push('\n');
    }

    let status = child.wait()
        .map_err(|e| anyhow!("falha aguardando CLI '{cli_cmd}': {e}"))?;

    if !status.success() {
        return Err(anyhow!("CLI '{cli_cmd}' retornou exit code {}", status));
    }

    Ok(full_output)
}
