use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Result};
use colored::Colorize;

use crate::core::session::PtySession;

/// CLIs conhecidos e como passam o prompt (via stdin)
const KNOWN_CLIS: &[(&str, &str)] = &[
    ("gemini", "Google Gemini CLI"),
    ("claude", "Anthropic Claude CLI"),
    ("grok", "Grok Build"),
    ("llm", "Simon Willison's LLM CLI"),
    ("aichat", "aichat CLI"),
    ("tgpt", "tgpt CLI"),
    ("sgpt", "ShellGPT CLI"),
    ("copilot", "GitHub Copilot CLI"),
];

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn temp_prompt_path(cli_id: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "ai-orchestrator-{}-prompt-{}-{}.md",
        cli_id,
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    path
}

/// Detecta quais CLIs conhecidos estão instalados no PATH.
pub fn detect_available_clis() -> Vec<(&'static str, &'static str)> {
    KNOWN_CLIS
        .iter()
        .filter(|(cmd, _)| is_cli_available(cmd))
        .copied()
        .collect()
}

/// Verifica se um CLI está disponível no PATH.
/// Valida apenas o primeiro token, de modo que comandos com subcomando/flags
/// (ex.: `codex exec`, `llm -m gpt-4o`) sejam aceitos — o `which` roda só sobre
/// o binário.
pub fn is_cli_available(cmd: &str) -> bool {
    let Some(bin) = cmd.split_whitespace().next() else {
        return false;
    };
    Command::new("which")
        .arg(bin)
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
        println!(
            "{}",
            format!("⚠ Nenhum CLI conhecido encontrado no PATH para o papel de {role}.").yellow()
        );
        println!(
            "  CLIs suportados: {}",
            KNOWN_CLIS
                .iter()
                .map(|(c, _)| *c)
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!(
            "  Você pode digitar o comando manualmente ou pressionar Enter para usar modo manual."
        );
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
        return Ok(Some(crate::core::cli_adapter::resolve_command(trimmed)));
    }

    println!("\n{}", format!("Escolha o CLI para IA {role}:").bold());
    for (i, (cmd, desc)) in available.iter().enumerate() {
        println!("  [{}] {} — {}", i + 1, cmd.cyan(), desc);
    }
    println!("  [{}] Outro (digitar comando)", available.len() + 1);
    println!(
        "  [{}] Modo manual (você cola o prompt)",
        available.len() + 2
    );
    print!("  Opção: ");
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input.trim().parse().unwrap_or(0);

    if choice >= 1 && choice <= available.len() {
        let cmd = crate::core::cli_adapter::resolve_command(available[choice - 1].0);
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
        let resolved = crate::core::cli_adapter::resolve_command(&trimmed);
        println!("  {} selecionado: {}", role, resolved.green());
        return Ok(Some(resolved));
    }

    // Modo manual
    Ok(None)
}

/// Envia o prompt para o CLI via stdin e exibe a resposta em streaming (linha a linha),
/// acumulando tudo para retornar ao final.
/// Se `log` for Some, envia as linhas pelo canal em vez de imprimir no stdout.
pub fn run_cli(
    cli_cmd: &str,
    prompt: &str,
    log: Option<crate::core::stream::LogTx>,
) -> Result<String> {
    run_cli_impl(cli_cmd, prompt, log, None)
}

pub fn run_cli_in_dir(
    cli_cmd: &str,
    prompt: &str,
    log: Option<crate::core::stream::LogTx>,
    cwd: &Path,
) -> Result<String> {
    run_cli_impl(cli_cmd, prompt, log, Some(cwd))
}

fn run_cli_impl(
    cli_cmd: &str,
    prompt: &str,
    log: Option<crate::core::stream::LogTx>,
    cwd: Option<&Path>,
) -> Result<String> {
    // Resolve o adapter (se for uma CLI conhecida) para aplicar env extra e,
    // ao final, limpar o stdout antes de devolver ao parser do agente.
    let adapter = crate::core::cli_adapter::adapter_for_command(cli_cmd);

    let mut prompt_file: Option<std::path::PathBuf> = None;
    let effective_cli_cmd = if let Some(ref a) = adapter {
        if let Some(arg) = a.prompt_file_arg() {
            let path = temp_prompt_path(a.id());
            std::fs::write(&path, prompt)?;
            prompt_file = Some(path.clone());
            format!(
                "{} {} {}",
                cli_cmd,
                arg,
                shell_quote(&path.display().to_string())
            )
        } else {
            cli_cmd.to_string()
        }
    } else {
        cli_cmd.to_string()
    };

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(&effective_cli_cmd)
        .env("GEMINI_CLI_TRUST_WORKSPACE", "true") // legado, inofensivo
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if let Some(ref a) = adapter {
        for (k, v) in a.env() {
            command.env(k, v);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|e| anyhow!("falha ao executar CLI '{effective_cli_cmd}': {e}"))?;

    if prompt_file.is_none() {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes())?;
        }
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("stdout não disponível para '{effective_cli_cmd}'"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("stderr não disponível para '{effective_cli_cmd}'"))?;

    let stderr_log = log.clone();
    let stderr_reader = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut stderr_output = String::new();
        for line in reader.lines().map_while(|line| line.ok()) {
            let is_visual_warning =
                line.contains("256-color support not detected") || line.trim().is_empty();
            if !is_visual_warning {
                if let Some(ref tx) = stderr_log {
                    let _ = tx.send(crate::core::stream::LogEvent::Line(format!(
                        "stderr: {}",
                        line
                    )));
                } else {
                    eprintln!("  {}", line);
                }
            }
            stderr_output.push_str(&line);
            stderr_output.push('\n');
        }
        stderr_output
    });

    let reader = BufReader::new(stdout);
    let mut full_output = String::new();

    for line in reader.lines() {
        let line = line?;
        if let Some(ref tx) = log {
            let _ = tx.send(crate::core::stream::LogEvent::Line(line.clone()));
        } else {
            println!("  {}", line);
        }
        full_output.push_str(&line);
        full_output.push('\n');
    }

    let status = child
        .wait()
        .map_err(|e| anyhow!("falha aguardando CLI '{effective_cli_cmd}': {e}"))?;
    cleanup_prompt_file(prompt_file.as_deref());
    let stderr_output = stderr_reader
        .join()
        .unwrap_or_else(|_| "falha lendo stderr do CLI".to_string());

    if !status.success() {
        let stderr_msg = stderr_output.trim();
        if stderr_msg.is_empty() {
            return Err(anyhow!(
                "CLI '{effective_cli_cmd}' retornou exit code {}",
                status
            ));
        }
        return Err(anyhow!(
            "CLI '{effective_cli_cmd}' retornou exit code {}: {}",
            status,
            stderr_msg
        ));
    }

    // Limpeza de saída específica do adapter (ex.: trim); identidade se desconhecido.
    let output = match adapter {
        Some(a) => a.clean_output(&full_output),
        None => full_output,
    };
    Ok(output)
}

fn cleanup_prompt_file(path: Option<&Path>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

/// Envia o prompt para uma sessão PTY existente e lê a resposta até detectar que a IA parou de escrever.
/// A sessão permanece ativa (em background) para próximos turnos.
pub fn run_cli_session(session: &mut PtySession, prompt: &str, timeout_ms: u64) -> Result<String> {
    // Envia o prompt para a sessão ativa
    session.send(prompt)?;

    // Como CLIs de IA em PTY costumam ser iterativos, eles imprimem as respostas e depois exibem um novo
    // prompt de entrada (ex: "❯ " ou "Claude> ") aguardando o usuário.
    // Em V1, vamos ler e drenar o output periodicamente. Retornamos quando o output ficar ocioso por um
    // tempo mínimo após começar a chegar, ou se atingir um limite.

    let mut full_output = String::new();
    let mut idle_count = 0;
    let mut wait_count = 0;
    const MAX_WAIT_CYCLES: u64 = 30; // Limite de espera (ex: 30 * timeout_ms)

    // Loop de leitura não-bloqueante (drenagem)
    loop {
        let chunk = session.read_output(timeout_ms)?;

        if !chunk.is_empty() {
            print!("{}", chunk.dimmed()); // Imprime chunk purificado para o usuário acompanhar
            std::io::stdout().flush().ok();
            full_output.push_str(&chunk);
            idle_count = 0; // reset
            wait_count = 0; // reset na espera inicial
        } else {
            idle_count += 1;
            // Se já leu alguma coisa e o buffer secou (idle por 3 ciclos de timeout), assumimos que a IA terminou o turno.
            if !full_output.is_empty() && idle_count >= 3 {
                break;
            }

            wait_count += 1;
            if wait_count >= MAX_WAIT_CYCLES {
                anyhow::bail!("Timeout: IA não respondeu após {} ciclos", MAX_WAIT_CYCLES);
            }
        }
    }
    println!(); // Quebra de linha final para a TUI do orquestrador

    Ok(full_output)
}
