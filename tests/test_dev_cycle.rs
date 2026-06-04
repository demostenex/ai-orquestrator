/// Integração E2E: ciclo completo dev → auditoria usando MockProvider e repo git temporário.
///
/// Cada teste simula uma rodada completa do loop:
///   1. MockProvider (dev) retorna um DevResponse JSON
///   2. O diff passa pela validação de segurança e parse
///   3. MockProvider (auditor) recebe o diff e retorna AuditResponse JSON
///   4. O resultado final é verificado
mod common;

use ai_orchestrator::agents::{auditor, dev};
use ai_orchestrator::core::{patch, security};

use common::{make_temp_git_repo, MockProvider};

// ── helpers locais ────────────────────────────────────────────────────────────

/// Retorna um diff mínimo aplicável a src/lib.rs (arquivo já existe no repo de teste).
fn valid_diff() -> String {
    "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-fn hello() {}\n+fn hello() { println!(\"hello\"); }\n".to_string()
}

fn dev_json(diff: &str) -> String {
    serde_json::json!({
        "step_id": "001",
        "summary": "Adds print to hello",
        "files_touched": ["src/lib.rs"],
        "diff": diff,
        "tests_suggested": ["cargo test"],
        "risks": []
    })
    .to_string()
}

fn audit_json(approved: bool) -> String {
    serde_json::json!({
        "approved": approved,
        "score": if approved { 95 } else { 30 },
        "problems": if approved { serde_json::json!([]) } else { serde_json::json!(["poor quality"]) },
        "required_changes": if approved { serde_json::json!([]) } else { serde_json::json!(["fix it"]) },
        "blocked_reason": if approved { serde_json::Value::Null } else { serde_json::json!("quality too low") }
    })
    .to_string()
}

// ── cenário 1: ciclo aprovado ─────────────────────────────────────────────────

/// Dev gera diff válido → segurança ok → auditoria aprova → patch aplicável.
#[tokio::test]
async fn cycle_approved_end_to_end() {
    let repo = make_temp_git_repo();
    let diff = valid_diff();

    // Passo 1: Dev produz resposta
    let dev_provider = MockProvider::returning(dev_json(&diff));
    let dev_call = dev::execute(
        &dev_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        "### src/lib.rs\n```\nfn hello() {}\n```\n",
    )
    .await
    .expect("dev execute failed");

    assert_eq!(dev_call.parsed.step_id, "001");
    assert!(!dev_call.parsed.diff.is_empty());

    // Passo 2: Verificação de segurança
    let violations = security::scan_diff(&dev_call.parsed.diff);
    assert!(
        violations.is_empty(),
        "unexpected security violations: {violations:?}"
    );

    // Passo 3: Parse do diff
    let parsed_diff = patch::parse_diff(&dev_call.parsed.diff).expect("parse_diff failed");
    assert!(parsed_diff.is_valid_unified_diff);
    assert!(parsed_diff
        .files_modified
        .contains(&"src/lib.rs".to_string()));

    // Passo 4: git apply --check
    let check = std::process::Command::new("git")
        .args(["apply", "--check", "-"])
        .current_dir(repo.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    use std::io::Write;
    let mut child = check;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(diff.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "git apply --check failed");

    let apply_result = if output.status.success() {
        "ok"
    } else {
        "failed"
    };

    // Passo 5: Auditoria aprova
    let audit_provider = MockProvider::returning(audit_json(true));
    let audit_call = auditor::execute(
        &audit_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        &dev_call.parsed.diff,
        &parsed_diff.files_modified,
        apply_result,
    )
    .await
    .expect("auditor execute failed");

    assert!(audit_call.parsed.approved);
    assert_eq!(audit_call.parsed.score, 95);
    assert!(audit_call.parsed.problems.is_empty());
}

// ── cenário 2: ciclo reprovado ────────────────────────────────────────────────

/// Dev gera diff válido → auditoria rejeita → blocked_reason presente.
#[tokio::test]
async fn cycle_rejected_by_auditor() {
    let diff = valid_diff();

    let dev_provider = MockProvider::returning(dev_json(&diff));
    let dev_call = dev::execute(
        &dev_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        "(workspace)",
    )
    .await
    .expect("dev execute failed");

    let violations = security::scan_diff(&dev_call.parsed.diff);
    assert!(violations.is_empty());

    let parsed_diff = patch::parse_diff(&dev_call.parsed.diff).expect("parse_diff failed");

    let audit_provider = MockProvider::returning(audit_json(false));
    let audit_call = auditor::execute(
        &audit_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        &dev_call.parsed.diff,
        &parsed_diff.files_modified,
        "ok",
    )
    .await
    .expect("auditor execute failed");

    assert!(!audit_call.parsed.approved);
    assert_eq!(audit_call.parsed.score, 30);
    assert!(!audit_call.parsed.problems.is_empty());
    assert!(audit_call.parsed.blocked_reason.is_some());
}

// ── cenário 3: segredo no diff bloqueia antes da auditoria ───────────────────

/// Diff com API_KEY → security::scan_diff detecta → auditoria nunca é chamada.
#[tokio::test]
async fn cycle_blocked_by_security_scan() {
    let diff_with_secret =
        "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-fn hello() {}\n+const API_KEY: &str = \"secret123\";\n";

    let dev_provider = MockProvider::returning(dev_json(diff_with_secret));
    let dev_call = dev::execute(
        &dev_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        "(workspace)",
    )
    .await
    .expect("dev execute failed");

    let violations = security::scan_diff(&dev_call.parsed.diff);

    // O ciclo deve ser interrompido aqui — auditoria não é chamada
    assert!(
        !violations.is_empty(),
        "security scan should have caught API_KEY"
    );
    assert!(violations.iter().any(|v| v.pattern == "API_KEY"));
}

// ── cenário 4: diff inválido falha no parse antes da auditoria ───────────────

/// DevResponse com diff malformado → parse_diff retorna Err → auditoria nunca é chamada.
#[tokio::test]
async fn cycle_invalid_diff_fails_before_audit() {
    let malformed_diff = "not a unified diff at all";
    let dev_provider = MockProvider::returning(dev_json(malformed_diff));

    let dev_call = dev::execute(
        &dev_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        "(workspace)",
    )
    .await
    .expect("dev execute itself should succeed — parse happens next");

    // parse_diff deve rejeitar o diff malformado
    let result = patch::parse_diff(&dev_call.parsed.diff);
    assert!(result.is_err(), "parse_diff should fail on malformed diff");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not a valid unified diff"), "got: {err}");
}

// ── cenário 5: provider do dev falha → ciclo aborta antes da auditoria ───────

/// MockProvider::failing() → dev::execute retorna Err → sem auditoria.
#[tokio::test]
async fn cycle_aborts_when_dev_provider_fails() {
    let dev_provider = MockProvider::failing();

    let result = dev::execute(
        &dev_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        "(workspace)",
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("mock provider failure"), "got: {err}");
}

// ── cenário 6: provider do auditor falha → erro propagado ────────────────────

/// Dev ok, mas MockProvider do auditor falha → auditor::execute retorna Err.
#[tokio::test]
async fn cycle_aborts_when_auditor_provider_fails() {
    let diff = valid_diff();
    let dev_provider = MockProvider::returning(dev_json(&diff));

    let dev_call = dev::execute(
        &dev_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        "(workspace)",
    )
    .await
    .expect("dev execute failed");

    let parsed_diff = patch::parse_diff(&dev_call.parsed.diff).expect("parse_diff failed");

    let audit_provider = MockProvider::failing();
    let result = auditor::execute(
        &audit_provider,
        "001",
        "Implement feature",
        "# Memory",
        None,
        &dev_call.parsed.diff,
        &parsed_diff.files_modified,
        "ok",
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("mock provider failure"), "got: {err}");
}
