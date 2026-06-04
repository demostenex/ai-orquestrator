mod common;

use ai_orchestrator::core::handoff::{
    create_auditor_to_dev_handoff, create_dev_to_auditor_handoff, load_last_handoff, save_handoff,
};
use ai_orchestrator::schemas::HandoffStatus;
use tempfile::TempDir;

// ── create_dev_to_auditor_handoff ─────────────────────────────────────────────

#[test]
fn dev_to_auditor_handoff_fields_are_correct() {
    let dev_resp = common::make_dev_response(&common::make_valid_diff("src/lib.rs"));
    let handoff = create_dev_to_auditor_handoff("run-001", "step-001", &dev_resp, "patchhash123");

    assert_eq!(handoff.agent, "dev");
    assert_eq!(handoff.target_agent, "auditor");
    assert_eq!(handoff.step_id, "step-001");
    assert_eq!(handoff.status, HandoffStatus::WaitingAudit);
    assert_eq!(handoff.summary, dev_resp.summary);
    assert_eq!(handoff.files_touched, dev_resp.files_touched);
    assert_eq!(handoff.risks, dev_resp.risks);
    assert!(handoff.decisions.iter().any(|d| d.contains("patchhash123")));
}

#[test]
fn dev_to_auditor_handoff_includes_suggested_tests_in_decisions() {
    let mut dev_resp = common::make_dev_response(&common::make_valid_diff("src/lib.rs"));
    dev_resp.tests_suggested = vec!["cargo test".to_string(), "cargo clippy".to_string()];

    let handoff = create_dev_to_auditor_handoff("run-001", "step-001", &dev_resp, "hash");
    let decisions_text = handoff.decisions.join(" ");
    assert!(decisions_text.contains("cargo test"));
}

// ── create_auditor_to_dev_handoff ─────────────────────────────────────────────

#[test]
fn auditor_to_dev_handoff_status_is_rejected() {
    let audit_resp = common::make_audit_response(false);
    let handoff = create_auditor_to_dev_handoff("run-001", "step-001", &audit_resp);

    assert_eq!(handoff.agent, "auditor");
    assert_eq!(handoff.target_agent, "dev");
    assert_eq!(handoff.status, HandoffStatus::Rejected);
    assert!(!handoff.next_action.is_empty());
}

#[test]
fn auditor_to_dev_handoff_propagates_required_changes() {
    let audit_resp = common::make_audit_response(false);
    let handoff = create_auditor_to_dev_handoff("run-001", "step-001", &audit_resp);

    assert!(!handoff.decisions.is_empty());
    assert!(!handoff.open_questions.is_empty());
}

// ── save_handoff / load_last_handoff ──────────────────────────────────────────

#[test]
fn save_and_load_handoff_roundtrip() {
    let dir = TempDir::new().unwrap();
    let orch = dir.path().join(".ai-orchestrator");
    std::fs::create_dir_all(orch.join("handoffs")).unwrap();

    let original = common::make_handoff("dev", "auditor", HandoffStatus::WaitingAudit);
    save_handoff(&orch, "run-001", "dev", "auditor", &original).unwrap();

    let loaded = load_last_handoff(&orch)
        .unwrap()
        .expect("should find handoff");
    assert_eq!(loaded.agent, original.agent);
    assert_eq!(loaded.target_agent, original.target_agent);
    assert_eq!(loaded.step_id, original.step_id);
    assert_eq!(loaded.status, original.status);
    assert_eq!(loaded.summary, original.summary);
    assert_eq!(loaded.files_touched, original.files_touched);
}

#[test]
fn load_last_handoff_returns_none_when_no_dir() {
    let dir = TempDir::new().unwrap();
    let orch = dir.path().join(".ai-orchestrator");
    // Não cria o diretório handoffs

    let result = load_last_handoff(&orch).unwrap();
    assert!(result.is_none());
}

#[test]
fn load_last_handoff_returns_most_recent() {
    let dir = TempDir::new().unwrap();
    let orch = dir.path().join(".ai-orchestrator");
    std::fs::create_dir_all(orch.join("handoffs")).unwrap();

    let h1 = common::make_handoff("dev", "auditor", HandoffStatus::WaitingAudit);
    save_handoff(&orch, "run-001", "dev", "auditor", &h1).unwrap();

    // Pequena espera para garantir timestamp diferente no filesystem
    std::thread::sleep(std::time::Duration::from_millis(20));

    let mut h2 = common::make_handoff("auditor", "dev", HandoffStatus::Rejected);
    h2.summary = "Second handoff — from auditor".to_string();
    save_handoff(&orch, "run-001", "auditor", "dev", &h2).unwrap();

    let loaded = load_last_handoff(&orch)
        .unwrap()
        .expect("should find handoff");
    assert_eq!(loaded.agent, "auditor");
    assert_eq!(loaded.summary, "Second handoff — from auditor");
}

#[test]
fn save_handoff_fails_on_invalid_handoff() {
    let dir = TempDir::new().unwrap();
    let orch = dir.path().join(".ai-orchestrator");
    std::fs::create_dir_all(orch.join("handoffs")).unwrap();

    let mut invalid = common::make_handoff("dev", "auditor", HandoffStatus::WaitingAudit);
    invalid.agent = "".to_string(); // agent vazio é inválido

    let result = save_handoff(&orch, "run-001", "dev", "auditor", &invalid);
    assert!(result.is_err());
}

#[test]
fn load_last_handoff_ignores_non_json_files() {
    let dir = TempDir::new().unwrap();
    let orch = dir.path().join(".ai-orchestrator");
    std::fs::create_dir_all(orch.join("handoffs")).unwrap();

    // Arquivos que devem ser ignorados
    std::fs::write(orch.join("handoffs").join("notes.txt"), "some text").unwrap();
    std::fs::write(orch.join("handoffs").join("README.md"), "# readme").unwrap();

    let result = load_last_handoff(&orch).unwrap();
    assert!(result.is_none());
}
