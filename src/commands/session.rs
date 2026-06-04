use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub pid: u32,
    pub agent_name: String,
    pub command: Vec<String>,
}

pub type SessionStore = HashMap<u32, SessionInfo>;

fn sessions_path(orchestrator_dir: &Path) -> std::path::PathBuf {
    orchestrator_dir.join("sessions.json")
}

pub fn load_sessions(orchestrator_dir: &Path) -> Result<SessionStore> {
    let path = sessions_path(orchestrator_dir);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

pub fn save_sessions(orchestrator_dir: &Path, sessions: &SessionStore) -> Result<()> {
    let content = serde_json::to_string_pretty(sessions)?;
    fs::write(sessions_path(orchestrator_dir), content)?;
    Ok(())
}

pub async fn list() -> Result<()> {
    let config = match crate::core::config::Config::load() {
        Ok(c) => c,
        Err(_) => {
            println!("Run `ai-orchestrator init` first");
            return Ok(());
        }
    };
    let sessions = load_sessions(&config.orchestrator_dir)?;
    if sessions.is_empty() {
        println!("Nenhuma sessão PTY ativa registrada.");
    } else {
        println!("{:<10} {:<20} COMANDO", "PID", "AGENTE");
        println!("{:<10} {:<20} -------", "---", "------");
        for (pid, info) in sessions {
            println!(
                "{:<10} {:<20} {}",
                pid,
                info.agent_name,
                info.command.join(" ")
            );
        }
    }
    Ok(())
}

pub async fn kill(pid: u32) -> Result<()> {
    let config = match crate::core::config::Config::load() {
        Ok(c) => c,
        Err(_) => {
            println!("Run `ai-orchestrator init` first");
            return Ok(());
        }
    };
    let mut sessions = load_sessions(&config.orchestrator_dir)?;
    if sessions.remove(&pid).is_some() {
        println!("Tentando encerrar processo com PID: {}", pid);
        #[cfg(unix)]
        {
            use nix::errno::Errno;
            use nix::sys::signal::{kill as kill_proc, Signal};
            use nix::unistd::Pid;
            match kill_proc(Pid::from_raw(pid as i32), Signal::SIGTERM) {
                Ok(()) => {}
                Err(Errno::ESRCH) => {
                    println!("  (processo já não existia mais)");
                }
                Err(e) => return Err(anyhow!("Falha ao enviar sinal para o processo: {}", e)),
            }
        }

        save_sessions(&config.orchestrator_dir, &sessions)?;
        println!("Sessão com PID {} removida do registro.", pid);
    } else {
        println!("Nenhuma sessão encontrada com PID {}.", pid);
    }
    Ok(())
}
