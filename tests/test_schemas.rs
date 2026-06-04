use ai_orchestrator::schemas::{AuditResponse, CycleState, DevResponse, Handoff, HandoffStatus};
use chrono::Utc;

#[test]
fn valid_dev_response_deserializes() {
    let raw = r#"{
        "step_id": "001",
        "summary": "Implement feature",
        "files_touched": ["src/main.rs"],
        "diff": "--- a/src/main.rs\n+++ b/src/main.rs\n@@\n-old\n+new",
        "tests_suggested": ["cargo test"],
        "risks": []
    }"#;

    let response: DevResponse = serde_json::from_str(raw).expect("json should deserialize");
    response.validate().expect("response should be valid");
}

#[test]
fn dev_response_with_empty_diff_fails_validation() {
    let raw = r#"{
        "step_id": "001",
        "summary": "Implement feature",
        "files_touched": ["src/main.rs"],
        "diff": "",
        "tests_suggested": [],
        "risks": []
    }"#;

    let response: DevResponse = serde_json::from_str(raw).expect("json should deserialize");
    assert!(response.validate().is_err());
}

#[test]
fn valid_audit_response_deserializes() {
    let raw = r#"{
        "approved": true,
        "score": 92,
        "problems": [],
        "required_changes": [],
        "blocked_reason": null
    }"#;

    let response: AuditResponse = serde_json::from_str(raw).expect("json should deserialize");
    response.validate().expect("response should be valid");
}

#[test]
fn audit_response_with_score_over_100_fails_validation() {
    let raw = r#"{
        "approved": false,
        "score": 101,
        "problems": [],
        "required_changes": [],
        "blocked_reason": null
    }"#;

    let response: AuditResponse = serde_json::from_str(raw).expect("json should deserialize");
    assert!(response.validate().is_err());
}

#[test]
fn valid_handoff_deserializes() {
    let raw = r#"{
        "agent": "dev",
        "target_agent": "auditor",
        "step_id": "001",
        "status": "waiting_audit",
        "summary": "Patch ready",
        "decisions": [],
        "files_touched": ["src/main.rs"],
        "open_questions": [],
        "risks": [],
        "next_action": "Audit patch"
    }"#;

    let handoff: Handoff = serde_json::from_str(raw).expect("json should deserialize");
    handoff.validate().expect("handoff should validate");
}

#[test]
fn handoff_with_invalid_status_fails() {
    let raw = r#"{
        "agent": "dev",
        "target_agent": "auditor",
        "step_id": "001",
        "status": "invalid",
        "summary": "Patch ready",
        "decisions": [],
        "files_touched": [],
        "open_questions": [],
        "risks": [],
        "next_action": "Audit patch"
    }"#;

    assert!(serde_json::from_str::<Handoff>(raw).is_err());
}

// ── Expansões de schemas ──────────────────────────────────────────────────────

#[test]
fn dev_response_empty_step_id_fails_validation() {
    let raw = r#"{
        "step_id": "",
        "summary": "something",
        "files_touched": [],
        "diff": "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n+new",
        "tests_suggested": [],
        "risks": []
    }"#;
    let response: DevResponse = serde_json::from_str(raw).unwrap();
    assert!(response.validate().is_err());
}

#[test]
fn audit_rejected_without_blocked_reason_is_valid() {
    let raw = r#"{
        "approved": false,
        "score": 30,
        "problems": ["Missing tests"],
        "required_changes": ["Add tests"],
        "blocked_reason": null
    }"#;
    let response: AuditResponse = serde_json::from_str(raw).unwrap();
    assert!(response.validate().is_ok());
}

#[test]
fn audit_score_zero_is_valid() {
    let raw = r#"{
        "approved": false,
        "score": 0,
        "problems": ["Everything is wrong"],
        "required_changes": [],
        "blocked_reason": "Total failure"
    }"#;
    let response: AuditResponse = serde_json::from_str(raw).unwrap();
    assert!(response.validate().is_ok());
}

#[test]
fn handoff_empty_agent_fails_validation() {
    let raw = r#"{
        "agent": "",
        "target_agent": "auditor",
        "step_id": "001",
        "status": "waiting_audit",
        "summary": "summary",
        "decisions": [],
        "files_touched": [],
        "open_questions": [],
        "risks": [],
        "next_action": "review"
    }"#;
    let handoff: Handoff = serde_json::from_str(raw).unwrap();
    assert!(handoff.validate().is_err());
}

#[test]
fn handoff_status_serde_roundtrip() {
    let statuses = [
        (HandoffStatus::WaitingAudit, "waiting_audit"),
        (HandoffStatus::Approved, "approved"),
        (HandoffStatus::Rejected, "rejected"),
        (HandoffStatus::ReadyForDev, "ready_for_dev"),
        (HandoffStatus::WaitingHuman, "waiting_human"),
    ];
    for (variant, expected_str) in statuses {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, format!("\"{expected_str}\""));
        let back: HandoffStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn cycle_state_empty_run_id_fails_validation() {
    let state = CycleState {
        run_id: "".to_string(),
        step_id: "step-001".to_string(),
        status: ai_orchestrator::schemas::CycleStatus::Initialized,
        base_commit: "abc123".to_string(),
        plan_hash: "planhash".to_string(),
        memory_hash: "memhash".to_string(),
        patch_file: None,
        patch_hash: None,
        audit_file: None,
        audit_approved: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    assert!(state.validate().is_err());
}

#[test]
fn cycle_state_empty_base_commit_fails_validation() {
    let state = CycleState {
        run_id: "run-001".to_string(),
        step_id: "step-001".to_string(),
        status: ai_orchestrator::schemas::CycleStatus::Initialized,
        base_commit: "".to_string(),
        plan_hash: "planhash".to_string(),
        memory_hash: "memhash".to_string(),
        patch_file: None,
        patch_hash: None,
        audit_file: None,
        audit_approved: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    assert!(state.validate().is_err());
}
