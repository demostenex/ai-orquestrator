use ai_orchestrator::schemas::{AuditResponse, DevResponse, Handoff};

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
