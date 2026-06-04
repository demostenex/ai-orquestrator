mod common;

use ai_orchestrator::core::db::EventType;

// ── Abertura e schema ─────────────────────────────────────────────────────────

#[tokio::test]
async fn db_opens_and_creates_schema() {
    let (_dir, db) = common::make_temp_db().await;
    // Se chegou aqui sem panic, o schema foi criado com sucesso
    assert!(!db.run_id().is_empty());
    assert!(!db.project_id().is_empty());
}

#[tokio::test]
async fn db_run_id_and_project_id_are_set() {
    let (_dir, db) = common::make_temp_db_with_run("my-run-xyz").await;
    assert_eq!(db.run_id(), "my-run-xyz");
    assert!(!db.project_id().is_empty());
}

// ── Planos ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn db_create_and_list_plans() {
    let (_dir, db) = common::make_temp_db().await;

    db.create_plan("plan-001", "Implement auth").await.unwrap();
    db.create_plan("plan-002", "Refactor DB layer")
        .await
        .unwrap();

    let plans = db.list_plans().await.unwrap();
    assert_eq!(plans.len(), 2);
    let ids: Vec<&str> = plans.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"plan-001"));
    assert!(ids.contains(&"plan-002"));
}

#[tokio::test]
async fn db_get_plan_title_returns_correct_title() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-abc", "My Test Plan").await.unwrap();

    let title = db.get_plan_title("plan-abc").await.unwrap();
    assert_eq!(title, "My Test Plan");
}

#[tokio::test]
async fn db_delete_plan_removes_plan_and_children() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-del", "To be deleted").await.unwrap();
    db.create_plan("plan-keep", "Survivor").await.unwrap();
    db.add_task("t-1", "plan-del", "task um", None, 1)
        .await
        .unwrap();
    db.add_plan_turn("plan-del", "architect", "prompt", "content")
        .await
        .unwrap();

    let removed = db.delete_plan("plan-del").await.unwrap();
    assert_eq!(removed, 1);

    // Plano sobrevivente permanece; o removido some.
    let plans = db.list_plans().await.unwrap();
    let ids: Vec<&str> = plans.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["plan-keep"]);

    // Filhos do plano removido foram apagados.
    assert!(db.get_plan_tasks("plan-del").await.unwrap().is_empty());
    assert!(db.get_plan_turns("plan-del").await.unwrap().is_empty());
}

#[tokio::test]
async fn db_delete_plan_returns_zero_when_absent() {
    let (_dir, db) = common::make_temp_db().await;
    let removed = db.delete_plan("nao-existe").await.unwrap();
    assert_eq!(removed, 0);
}

// ── Tarefas ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn db_add_and_get_tasks() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-1", "plan-001", "Setup project", None, 1)
        .await
        .unwrap();
    db.add_task("task-2", "plan-001", "Write tests", Some("dev"), 2)
        .await
        .unwrap();

    let tasks = db.get_plan_tasks("plan-001").await.unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id, "task-1");
    assert_eq!(tasks[0].status, "pending");
    assert_eq!(tasks[0].assigned_to, None);
    assert_eq!(tasks[1].id, "task-2");
    assert_eq!(tasks[1].assigned_to, Some("dev".to_string()));
}

#[tokio::test]
async fn db_tasks_are_ordered_by_sequence() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-c", "plan-001", "Third", None, 3)
        .await
        .unwrap();
    db.add_task("task-a", "plan-001", "First", None, 1)
        .await
        .unwrap();
    db.add_task("task-b", "plan-001", "Second", None, 2)
        .await
        .unwrap();

    let tasks = db.get_plan_tasks("plan-001").await.unwrap();
    assert_eq!(tasks[0].id, "task-a");
    assert_eq!(tasks[1].id, "task-b");
    assert_eq!(tasks[2].id, "task-c");
}

#[tokio::test]
async fn db_lock_plan_tasks_sets_write_locked() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-1", "plan-001", "Do something", None, 1)
        .await
        .unwrap();

    db.lock_plan_tasks("plan-001").await.unwrap();

    // Tentar atualizar como agente não-dev deve falhar
    let err = db
        .update_task_status("architect", "task-1", "completed")
        .await;
    assert!(err.is_err(), "architect should not update locked task");
    assert!(err.unwrap_err().to_string().contains("bloqueada"));
}

#[tokio::test]
async fn db_lock_does_not_block_dev_agent() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-1", "plan-001", "Do something", None, 1)
        .await
        .unwrap();

    db.lock_plan_tasks("plan-001").await.unwrap();

    // IA Dev pode atualizar mesmo após lock
    db.update_task_status("dev", "task-1", "completed")
        .await
        .unwrap();
    let tasks = db.get_plan_tasks("plan-001").await.unwrap();
    assert_eq!(tasks[0].status, "completed");
}

#[tokio::test]
async fn db_update_task_status_changes_status() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-1", "plan-001", "Do something", None, 1)
        .await
        .unwrap();

    db.update_task_status("dev", "task-1", "completed")
        .await
        .unwrap();

    let tasks = db.get_plan_tasks("plan-001").await.unwrap();
    assert_eq!(tasks[0].status, "completed");
}

// ── pick_next_task ────────────────────────────────────────────────────────────

#[tokio::test]
async fn db_pick_next_task_returns_first_pending() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-1", "plan-001", "First task", None, 1)
        .await
        .unwrap();
    db.add_task("task-2", "plan-001", "Second task", None, 2)
        .await
        .unwrap();

    let task = db.pick_next_task("plan-001").await.unwrap();
    assert!(task.is_some());
    let task = task.unwrap();
    assert_eq!(task.id, "task-1");

    // Após pick, deve estar in_progress
    let tasks = db.get_plan_tasks("plan-001").await.unwrap();
    assert_eq!(tasks[0].status, "in_progress");
    assert_eq!(tasks[1].status, "pending");
}

#[tokio::test]
async fn db_pick_next_task_returns_none_when_all_done() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-1", "plan-001", "First task", None, 1)
        .await
        .unwrap();

    db.update_task_status("dev", "task-1", "completed")
        .await
        .unwrap();

    let task = db.pick_next_task("plan-001").await.unwrap();
    assert!(task.is_none());
}

#[tokio::test]
async fn db_pick_next_task_respects_plan_isolation() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-A", "Plan A").await.unwrap();
    db.create_plan("plan-B", "Plan B").await.unwrap();
    db.add_task("task-a1", "plan-A", "Task in A", None, 1)
        .await
        .unwrap();
    db.add_task("task-b1", "plan-B", "Task in B", None, 1)
        .await
        .unwrap();

    let task_a = db.pick_next_task("plan-A").await.unwrap().unwrap();
    assert_eq!(task_a.id, "task-a1");

    let task_b = db.pick_next_task("plan-B").await.unwrap().unwrap();
    assert_eq!(task_b.id, "task-b1");
}

// ── reset_in_progress_tasks ───────────────────────────────────────────────────

#[tokio::test]
async fn db_reset_in_progress_tasks_returns_to_pending() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-1", "plan-001", "First task", None, 1)
        .await
        .unwrap();
    db.add_task("task-2", "plan-001", "Second task", None, 2)
        .await
        .unwrap();

    // Simula pick para marcar task-1 como in_progress
    db.pick_next_task("plan-001").await.unwrap();

    let count = db.reset_in_progress_tasks("plan-001").await.unwrap();
    assert_eq!(count, 1);

    let tasks = db.get_plan_tasks("plan-001").await.unwrap();
    assert_eq!(tasks[0].status, "pending");
    assert_eq!(tasks[1].status, "pending");
}

#[tokio::test]
async fn db_reset_returns_zero_when_nothing_in_progress() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();
    db.add_task("task-1", "plan-001", "Task", None, 1)
        .await
        .unwrap();

    let count = db.reset_in_progress_tasks("plan-001").await.unwrap();
    assert_eq!(count, 0);
}

// ── Eventos ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn db_log_and_list_events() {
    let (_dir, db) = common::make_temp_db().await;

    db.log_event(
        EventType::PromptSent,
        Some("orchestrator"),
        Some("dev"),
        Some("Sending prompt to dev"),
        None,
        false,
        None,
    )
    .await
    .unwrap();

    db.log_event(
        EventType::ResponseReceived,
        Some("dev"),
        Some("orchestrator"),
        Some("Received dev response"),
        Some("abc123"),
        false,
        None,
    )
    .await
    .unwrap();

    let events = db.list_events().await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "prompt_sent");
    assert_eq!(events[0].sequence, 1);
    assert_eq!(events[1].event_type, "response_received");
    assert_eq!(events[1].sequence, 2);
}

// ── Handoffs no banco ─────────────────────────────────────────────────────────

#[tokio::test]
async fn db_log_handoff_persists() {
    let (_dir, db) = common::make_temp_db().await;

    db.log_handoff(
        "dev",
        "auditor",
        "waiting_audit",
        "Patch ready for review",
        vec!["Use async/await".to_string()],
        vec!["Low risk".to_string()],
        vec![],
        vec!["src/lib.rs".to_string()],
        "Review the patch",
    )
    .await
    .unwrap();
    // Sem erro = persistido com sucesso
}

// ── update_run_status ─────────────────────────────────────────────────────────

#[tokio::test]
async fn db_update_run_status_persists() {
    let (_dir, db) = common::make_temp_db().await;

    db.update_run_status(
        "completed",
        Some("patchhash123"),
        Some(true),
        Some(95),
        None,
    )
    .await
    .unwrap();
    // Sem erro = status atualizado com sucesso
}

// ── Plan turns e versions ─────────────────────────────────────────────────────

#[tokio::test]
async fn db_plan_turns_roundtrip() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();

    db.add_plan_turn(
        "plan-001",
        "architect",
        "Analyze structure",
        "Here is the architecture...",
    )
    .await
    .unwrap();
    db.add_plan_turn("plan-001", "dev", "Detail tasks", "Task list: ...")
        .await
        .unwrap();

    let turns = db.get_plan_turns("plan-001").await.unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].sequence, 1);
    assert_eq!(turns[0].agent, "architect");
    assert_eq!(turns[0].prompt, "Analyze structure");
    assert_eq!(turns[1].sequence, 2);
    assert_eq!(turns[1].agent, "dev");
}

#[tokio::test]
async fn db_plan_version_and_latest_hash() {
    let (_dir, db) = common::make_temp_db().await;
    db.create_plan("plan-001", "Test Plan").await.unwrap();

    // Sem versão ainda
    let hash = db.get_latest_plan_hash("plan-001").await.unwrap();
    assert!(hash.is_none());

    db.add_plan_version("plan-001", "hash-v1", Some("Initial version"))
        .await
        .unwrap();
    db.add_plan_version("plan-001", "hash-v2", Some("User revised"))
        .await
        .unwrap();

    let latest = db.get_latest_plan_hash("plan-001").await.unwrap();
    assert_eq!(latest, Some("hash-v2".to_string()));
}
