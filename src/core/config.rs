use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: String,
    pub model: String,
    pub step_id: String,
    pub workspace_dir: PathBuf,
    pub orchestrator_dir: PathBuf,
    pub dev_cli: Option<String>,
    pub audit_cli: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConfig {
    pub provider: String,
    pub model: String,
    pub step_id: String,
    /// Comando CLI usado como IA Dev (ex: "gemini", "claude", "llm")
    #[serde(default)]
    pub dev_cli: Option<String>,
    /// Comando CLI usado como IA Auditora
    #[serde(default)]
    pub audit_cli: Option<String>,
}

impl Default for StoredConfig {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            step_id: "001".to_string(),
            dev_cli: None,
            audit_cli: None,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let workspace_dir = detect_workspace_dir()?;
        let orchestrator_dir = workspace_dir.join(".ai-orchestrator");
        let config_path = orchestrator_dir.join("config.json");
        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read config file {}", config_path.display()))?;
        let stored: StoredConfig = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse config file {}", config_path.display()))?;

        Ok(Self {
            provider: stored.provider,
            model: stored.model,
            step_id: stored.step_id,
            dev_cli: stored.dev_cli,
            audit_cli: stored.audit_cli,
            workspace_dir,
            orchestrator_dir,
        })
    }

    pub fn config_path(workspace_dir: &Path) -> PathBuf {
        workspace_dir.join(".ai-orchestrator").join("config.json")
    }
}

pub fn detect_workspace_dir() -> Result<PathBuf> {
    if let Ok(path) = env::var("ORCHESTRATOR_WORKSPACE") {
        return Ok(PathBuf::from(path));
    }

    let docker_workspace = PathBuf::from("/workspace");
    if docker_workspace.exists() {
        return Ok(docker_workspace);
    }

    env::current_dir().context("failed to detect current working directory")
}
