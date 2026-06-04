mod common;

use ai_orchestrator::core::memory::{load_last_sync, sync_handoff_summary};
use ai_orchestrator::schemas::HandoffStatus;
use tempfile::TempDir;

fn make_orch_with_memory() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let orch = common::make_orch_dirs(&dir);
    (dir, orch)
}

// ── sync_handoff_summary ──────────────────────────────────────────────────────

#[test]
fn sync_creates_context_file_if_missing() {
    let dir = TempDir::new().unwrap();
    let orch = dir.path().join(".ai-orchestrator");
    std::fs::create_dir_all(orch.join("memory")).unwrap();
    std::fs::create_dir_all(orch.join("memory-sync")).unwrap();
    // Não cria context.md

    let handoff = common::make_handoff("dev", "auditor", HandoffStatus::WaitingAudit);
    sync_handoff_summary(&orch, "run-001", &handoff).unwrap();

    assert!(orch.join("memory").join("context.md").exists());
}

#[test]
fn sync_appends_to_existing_context() {
    let (_dir, orch) = make_orch_with_memory();

    let h1 = common::make_handoff("dev", "auditor", HandoffStatus::WaitingAudit);
    sync_handoff_summary(&orch, "run-001", &h1).unwrap();

    let h2 = common::make_handoff("auditor", "dev", HandoffStatus::Rejected);
    sync_handoff_summary(&orch, "run-002", &h2).unwrap();

    let content = std::fs::read_to_string(orch.join("memory").join("context.md")).unwrap();
    assert!(content.contains("run-001"));
    assert!(content.contains("run-002"));
}

#[test]
fn sync_returns_memory_hash() {
    let (_dir, orch) = make_orch_with_memory();

    let handoff = common::make_handoff("dev", "auditor", HandoffStatus::WaitingAudit);
    let hash = sync_handoff_summary(&orch, "run-001", &handoff).unwrap();

    assert!(
        hash.starts_with("sha256:"),
        "hash should be sha256: prefixed, got: {hash}"
    );
    assert_eq!(hash.len(), 71); // "sha256:" + 64 hex chars
}

#[test]
fn sync_creates_sync_state_file() {
    let (_dir, orch) = make_orch_with_memory();

    let handoff = common::make_handoff("dev", "auditor", HandoffStatus::WaitingAudit);
    let hash = sync_handoff_summary(&orch, "run-001", &handoff).unwrap();

    let sync_file = orch.join("memory-sync").join("last-sync.json");
    assert!(sync_file.exists());

    let content = std::fs::read_to_string(&sync_file).unwrap();
    assert!(content.contains(&hash));
}

#[test]
fn sync_all_handoff_status_variants_produce_labels() {
    let statuses = [
        (HandoffStatus::WaitingAudit, "Aguardando auditoria"),
        (HandoffStatus::Approved, "Aprovado"),
        (HandoffStatus::Rejected, "Reprovado"),
        (HandoffStatus::ReadyForDev, "Pronto para Dev"),
        (HandoffStatus::WaitingHuman, "Aguardando humano"),
    ];

    for (status, expected_label) in statuses {
        let dir = TempDir::new().unwrap();
        let orch = common::make_orch_dirs(&dir);

        let handoff = common::make_handoff("dev", "auditor", status);
        sync_handoff_summary(&orch, "run-001", &handoff).unwrap();

        let content = std::fs::read_to_string(orch.join("memory").join("context.md")).unwrap();
        assert!(
            content.contains(expected_label),
            "Expected label '{expected_label}' not found in context.md"
        );
    }
}

// ── load_last_sync ────────────────────────────────────────────────────────────

#[test]
fn load_last_sync_returns_none_when_missing() {
    let dir = TempDir::new().unwrap();
    let orch = dir.path().join(".ai-orchestrator");
    std::fs::create_dir_all(orch.join("memory-sync")).unwrap();
    // Não cria o arquivo last-sync.json

    let state = load_last_sync(&orch).unwrap();
    assert!(state.last_sync.is_none());
    assert!(state.memory_hash.is_none());
}

#[test]
fn load_last_sync_returns_state_after_sync() {
    let (_dir, orch) = make_orch_with_memory();

    let handoff = common::make_handoff("dev", "auditor", HandoffStatus::WaitingAudit);
    let hash = sync_handoff_summary(&orch, "run-001", &handoff).unwrap();

    let state = load_last_sync(&orch).unwrap();
    assert!(state.last_sync.is_some());
    assert_eq!(state.memory_hash, Some(hash));
}
