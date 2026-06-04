#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

use ai_orchestrator::core::db::Db;
use ai_orchestrator::providers::base::{ChatMessage, Provider, ProviderResponse};
use ai_orchestrator::schemas::{AuditResponse, DevResponse, Handoff, HandoffStatus};
use anyhow::Result;
use async_trait::async_trait;
use tempfile::TempDir;

// ── Banco de dados isolado ────────────────────────────────────────────────────

/// Cria um Db em diretório temporário isolado por teste.
pub async fn make_temp_db() -> (TempDir, Db) {
    let dir = TempDir::new().expect("tempdir");
    let orch_dir = dir.path().join(".ai-orchestrator");
    std::fs::create_dir_all(&orch_dir).expect("create orch dir");

    let db = Db::open(
        &orch_dir,
        dir.path(),
        "run-test-001",
        "step-001",
        "auto",
        "abc1234",
        "planhash",
        "memhash",
    )
    .expect("open db");

    (dir, db)
}

/// Cria um Db com run_id customizado.
pub async fn make_temp_db_with_run(run_id: &str) -> (TempDir, Db) {
    let dir = TempDir::new().expect("tempdir");
    let orch_dir = dir.path().join(".ai-orchestrator");
    std::fs::create_dir_all(&orch_dir).expect("create orch dir");

    let db = Db::open(
        &orch_dir,
        dir.path(),
        run_id,
        "step-001",
        "auto",
        "abc1234",
        "planhash",
        "memhash",
    )
    .expect("open db");

    (dir, db)
}

// ── Repositório git temporário ────────────────────────────────────────────────

/// Cria um repositório git isolado com um commit inicial contendo `src/lib.rs`.
pub fn make_temp_git_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir for git repo");
    let path = dir.path();

    run_git(path, &["init"]);
    run_git(path, &["config", "user.email", "test@test.com"]);
    run_git(path, &["config", "user.name", "Test"]);

    let src_dir = path.join("src");
    std::fs::create_dir_all(&src_dir).expect("create src dir");
    std::fs::write(src_dir.join("lib.rs"), "fn hello() {}\n").expect("write lib.rs");

    run_git(path, &["add", "-A"]);
    run_git(path, &["commit", "-m", "initial commit"]);

    dir
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("git command failed");
    assert!(status.success(), "git {:?} failed", args);
}

// ── Builders de fixtures ──────────────────────────────────────────────────────

/// Gera um Unified Diff mínimo válido para o arquivo informado.
pub fn make_valid_diff(file: &str) -> String {
    format!("--- a/{file}\n+++ b/{file}\n@@ -1 +1 @@\n-old line\n+new line\n")
}

/// Gera um DevResponse válido com o diff fornecido.
pub fn make_dev_response(diff: &str) -> DevResponse {
    DevResponse {
        step_id: "step-001".to_string(),
        summary: "Implements feature X".to_string(),
        files_touched: vec!["src/lib.rs".to_string()],
        diff: diff.to_string(),
        tests_suggested: vec!["cargo test".to_string()],
        risks: vec!["Low risk change".to_string()],
    }
}

/// Gera um AuditResponse com o resultado de aprovação informado.
pub fn make_audit_response(approved: bool) -> AuditResponse {
    AuditResponse {
        approved,
        score: if approved { 90 } else { 40 },
        problems: if approved {
            vec![]
        } else {
            vec!["Missing error handling".to_string()]
        },
        required_changes: if approved {
            vec![]
        } else {
            vec!["Add proper error handling".to_string()]
        },
        blocked_reason: if approved {
            None
        } else {
            Some("Code quality too low".to_string())
        },
    }
}

/// Gera um Handoff válido entre os agentes informados.
pub fn make_handoff(from: &str, to: &str, status: HandoffStatus) -> Handoff {
    Handoff {
        agent: from.to_string(),
        target_agent: to.to_string(),
        step_id: "step-001".to_string(),
        status,
        summary: "Test handoff summary".to_string(),
        decisions: vec!["Decision A".to_string()],
        files_touched: vec!["src/lib.rs".to_string()],
        open_questions: vec![],
        risks: vec!["Minor risk".to_string()],
        next_action: "Review the patch".to_string(),
    }
}

// ── MockProvider ──────────────────────────────────────────────────────────────

/// Provider fake para testes sem chamadas HTTP.
pub struct MockProvider {
    pub response: String,
    pub should_fail: bool,
}

impl MockProvider {
    pub fn returning(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            should_fail: false,
        }
    }

    pub fn failing() -> Self {
        Self {
            response: String::new(),
            should_fail: true,
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn complete(
        &self,
        _system: &str,
        _messages: Vec<ChatMessage>,
    ) -> Result<ProviderResponse> {
        if self.should_fail {
            anyhow::bail!("mock provider failure");
        }
        Ok(ProviderResponse {
            text: self.response.clone(),
            model: "mock-model".to_string(),
            finish_reason: "stop".to_string(),
            provider: "mock".to_string(),
        })
    }

    fn name(&self) -> &str {
        "mock"
    }
}

// ── Diretório de orquestrador temporário ─────────────────────────────────────

/// Cria a estrutura de diretórios esperada pelo orquestrador dentro de um TempDir.
pub fn make_orch_dirs(dir: &TempDir) -> PathBuf {
    let orch = dir.path().join(".ai-orchestrator");
    for sub in &["handoffs", "memory", "memory-sync", "patches"] {
        std::fs::create_dir_all(orch.join(sub)).expect("create orch subdirs");
    }
    // Cria context.md vazio para testes que precisam do arquivo
    std::fs::write(orch.join("memory").join("context.md"), "# AI Memory\n")
        .expect("write context.md");
    orch
}
