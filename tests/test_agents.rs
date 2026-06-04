mod common;

use ai_orchestrator::agents::{auditor, dev};
use ai_orchestrator::schemas::{HandoffStatus, Task};

// ── Dev: build_user_prompt ────────────────────────────────────────────────────

#[test]
fn dev_prompt_includes_step_id() {
    let prompt = dev::build_user_prompt("step-42", "plan content", "memory", None, "workspace");
    assert!(prompt.contains("step-42"));
}

#[test]
fn dev_prompt_includes_plan_content() {
    let prompt = dev::build_user_prompt("step-1", "MY PLAN CONTENT", "memory", None, "workspace");
    assert!(prompt.contains("MY PLAN CONTENT"));
}

#[test]
fn dev_prompt_includes_memory() {
    let prompt = dev::build_user_prompt("step-1", "plan", "MEMORY CONTEXT HERE", None, "workspace");
    assert!(prompt.contains("MEMORY CONTEXT HERE"));
}

#[test]
fn dev_prompt_includes_handoff_when_present() {
    let handoff = common::make_handoff("auditor", "dev", HandoffStatus::Rejected);
    let prompt = dev::build_user_prompt("step-1", "plan", "memory", Some(&handoff), "workspace");
    assert!(
        prompt.contains("auditor"),
        "prompt should contain handoff agent"
    );
    assert!(prompt.contains("Test handoff summary"));
}

#[test]
fn dev_prompt_shows_no_handoff_message_when_absent() {
    let prompt = dev::build_user_prompt("step-1", "plan", "memory", None, "workspace");
    assert!(prompt.contains("Nenhum handoff anterior"));
}

// ── Dev: build_dev_task_prompt ────────────────────────────────────────────────

fn make_tasks() -> Vec<Task> {
    vec![
        Task {
            id: "task-1".to_string(),
            description: "Setup database".to_string(),
            status: "completed".to_string(),
            assigned_to: None,
        },
        Task {
            id: "task-2".to_string(),
            description: "Implement auth".to_string(),
            status: "in_progress".to_string(),
            assigned_to: Some("dev".to_string()),
        },
        Task {
            id: "task-3".to_string(),
            description: "Write tests".to_string(),
            status: "pending".to_string(),
            assigned_to: None,
        },
    ]
}

#[test]
fn dev_task_prompt_marks_current_task() {
    let tasks = make_tasks();
    let current = tasks[1].clone();
    let prompt = dev::build_dev_task_prompt("My Plan", &tasks, &current, None);
    assert!(
        prompt.contains("⏳ CURRENT"),
        "current task should be marked"
    );
    assert!(prompt.contains("Implement auth"));
}

#[test]
fn dev_task_prompt_marks_completed_tasks() {
    let tasks = make_tasks();
    let current = tasks[1].clone();
    let prompt = dev::build_dev_task_prompt("My Plan", &tasks, &current, None);
    assert!(
        prompt.contains("✅ done"),
        "completed task should be marked"
    );
    assert!(prompt.contains("Setup database"));
}

#[test]
fn dev_task_prompt_marks_pending_tasks() {
    let tasks = make_tasks();
    let current = tasks[1].clone();
    let prompt = dev::build_dev_task_prompt("My Plan", &tasks, &current, None);
    assert!(
        prompt.contains("⬜ pending"),
        "pending task should be marked"
    );
}

#[test]
fn dev_task_prompt_includes_user_notes() {
    let tasks = make_tasks();
    let current = tasks[1].clone();
    let prompt =
        dev::build_dev_task_prompt("My Plan", &tasks, &current, Some("Focus on JWT tokens"));
    assert!(prompt.contains("Focus on JWT tokens"));
    assert!(prompt.contains("NOTAS DO HUMANO"));
}

#[test]
fn dev_task_prompt_omits_notes_section_when_empty() {
    let tasks = make_tasks();
    let current = tasks[1].clone();
    let prompt = dev::build_dev_task_prompt("My Plan", &tasks, &current, None);
    assert!(!prompt.contains("NOTAS DO HUMANO"));
}

#[test]
fn dev_task_prompt_omits_notes_when_whitespace_only() {
    let tasks = make_tasks();
    let current = tasks[1].clone();
    let prompt = dev::build_dev_task_prompt("My Plan", &tasks, &current, Some("   "));
    assert!(!prompt.contains("NOTAS DO HUMANO"));
}

#[test]
fn dev_task_prompt_includes_plan_title() {
    let tasks = make_tasks();
    let current = tasks[0].clone();
    let prompt = dev::build_dev_task_prompt("Grand Refactoring Plan", &tasks, &current, None);
    assert!(prompt.contains("Grand Refactoring Plan"));
}

// ── Dev: execute com MockProvider ─────────────────────────────────────────────

#[tokio::test]
async fn dev_execute_parses_valid_provider_response() {
    let diff = common::make_valid_diff("src/lib.rs");
    let json = format!(
        r#"{{
            "step_id": "step-001",
            "summary": "Adds hello function",
            "files_touched": ["src/lib.rs"],
            "diff": {diff:?},
            "tests_suggested": ["cargo test"],
            "risks": []
        }}"#
    );
    let provider = common::MockProvider::returning(json);
    let result = dev::execute(&provider, "step-001", "plan", "memory", None, "workspace").await;
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.parsed.step_id, "step-001");
    assert_eq!(call.parsed.summary, "Adds hello function");
}

#[tokio::test]
async fn dev_execute_fails_on_invalid_json() {
    let provider = common::MockProvider::returning("not json at all");
    let result = dev::execute(&provider, "step-001", "plan", "memory", None, "workspace").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn dev_execute_fails_when_provider_fails() {
    let provider = common::MockProvider::failing();
    let result = dev::execute(&provider, "step-001", "plan", "memory", None, "workspace").await;
    assert!(result.is_err());
}

// ── Auditor: build_user_prompt ────────────────────────────────────────────────

#[test]
fn auditor_prompt_includes_diff() {
    let diff = common::make_valid_diff("src/lib.rs");
    let prompt = auditor::build_user_prompt(
        "step-001",
        "plan",
        "memory",
        None,
        &diff,
        &["src/lib.rs".to_string()],
        "ok",
    );
    assert!(prompt.contains(&diff));
}

#[test]
fn auditor_prompt_includes_apply_check_result() {
    let prompt = auditor::build_user_prompt(
        "step-001",
        "plan",
        "memory",
        None,
        "diff content",
        &[],
        "APPLY CHECK OK",
    );
    assert!(prompt.contains("APPLY CHECK OK"));
}

#[test]
fn auditor_prompt_shows_none_when_no_files() {
    let prompt = auditor::build_user_prompt("step-001", "plan", "memory", None, "diff", &[], "ok");
    assert!(prompt.contains("(none)"));
}

#[test]
fn auditor_prompt_includes_files_modified() {
    let files = vec!["src/lib.rs".to_string(), "src/main.rs".to_string()];
    let prompt =
        auditor::build_user_prompt("step-001", "plan", "memory", None, "diff", &files, "ok");
    assert!(prompt.contains("src/lib.rs"));
    assert!(prompt.contains("src/main.rs"));
}

// ── Auditor: execute com MockProvider ─────────────────────────────────────────

#[tokio::test]
async fn auditor_execute_parses_valid_response() {
    let json = r#"{
        "approved": true,
        "score": 88,
        "problems": [],
        "required_changes": [],
        "blocked_reason": null
    }"#;
    let provider = common::MockProvider::returning(json);
    let result = auditor::execute(
        &provider,
        "step-001",
        "plan",
        "memory",
        None,
        "diff content",
        &[],
        "ok",
    )
    .await;
    assert!(result.is_ok());
    let call = result.unwrap();
    assert!(call.parsed.approved);
    assert_eq!(call.parsed.score, 88);
}

#[tokio::test]
async fn auditor_execute_fails_on_invalid_json() {
    let provider = common::MockProvider::returning("{ invalid json }");
    let result = auditor::execute(
        &provider,
        "step-001",
        "plan",
        "memory",
        None,
        "diff",
        &[],
        "ok",
    )
    .await;
    assert!(result.is_err());
}
