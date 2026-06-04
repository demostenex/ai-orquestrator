mod common;

use std::fs;

use ai_orchestrator::core::git;

// ── get_head_commit ───────────────────────────────────────────────────────────

#[test]
fn get_head_commit_returns_40_char_hash() {
    let dir = common::make_temp_git_repo();
    let commit = git::get_head_commit(dir.path()).unwrap();
    assert_eq!(
        commit.len(),
        40,
        "SHA-1 deve ter 40 caracteres, got: {commit}"
    );
    assert!(commit.chars().all(|c| c.is_ascii_hexdigit()));
}

// ── is_clean_tree ─────────────────────────────────────────────────────────────

#[test]
fn is_clean_tree_on_clean_repo() {
    let dir = common::make_temp_git_repo();
    let clean = git::is_clean_tree(dir.path()).unwrap();
    assert!(clean, "repo just created should be clean");
}

#[test]
fn is_clean_tree_on_dirty_repo() {
    let dir = common::make_temp_git_repo();

    // Cria arquivo não rastreado
    fs::write(dir.path().join("src").join("new_file.rs"), "fn dirty() {}").unwrap();

    let clean = git::is_clean_tree(dir.path()).unwrap();
    assert!(!clean, "repo with unstaged file should be dirty");
}

// ── apply_check ───────────────────────────────────────────────────────────────

#[test]
fn apply_check_passes_on_valid_patch() {
    let dir = common::make_temp_git_repo();
    let patch_path = dir.path().join("test.diff");

    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-fn hello() {}\n+fn hello() { println!(\"hello\"); }\n";
    fs::write(&patch_path, diff).unwrap();

    git::apply_check(dir.path(), &patch_path).unwrap();
}

#[test]
fn apply_check_fails_on_invalid_patch() {
    let dir = common::make_temp_git_repo();
    let patch_path = dir.path().join("bad.diff");

    // Patch que não corresponde ao conteúdo real do arquivo
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-THIS LINE DOES NOT EXIST\n+replacement\n";
    fs::write(&patch_path, diff).unwrap();

    let result = git::apply_check(dir.path(), &patch_path);
    assert!(result.is_err(), "invalid patch should fail apply_check");
}

// ── apply_patch ───────────────────────────────────────────────────────────────

#[test]
fn apply_patch_modifies_file() {
    let dir = common::make_temp_git_repo();
    let patch_path = dir.path().join("test.diff");

    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-fn hello() {}\n+fn hello() { println!(\"hello\"); }\n";
    fs::write(&patch_path, diff).unwrap();

    git::apply_patch(dir.path(), &patch_path).unwrap();

    let content = fs::read_to_string(dir.path().join("src").join("lib.rs")).unwrap();
    assert!(
        content.contains("println!"),
        "file should be modified after apply"
    );
}

// ── current_branch ────────────────────────────────────────────────────────────

#[test]
fn current_branch_returns_branch_name() {
    let dir = common::make_temp_git_repo();
    let branch = git::current_branch(dir.path()).unwrap();
    // Pode ser "main" ou "master" dependendo da config do git
    assert!(!branch.is_empty());
    assert!(branch == "main" || branch == "master" || !branch.is_empty());
}

// ── last_commit_summary ───────────────────────────────────────────────────────

#[test]
fn last_commit_summary_returns_hash_and_message() {
    let dir = common::make_temp_git_repo();
    let summary = git::last_commit_summary(dir.path()).unwrap();

    // Formato: "<short_hash> <message>"
    assert!(summary.contains("initial commit"), "got: {summary}");
    // Short hash tem pelo menos 7 chars antes do espaço
    let parts: Vec<&str> = summary.splitn(2, ' ').collect();
    assert_eq!(parts.len(), 2);
    assert!(parts[0].len() >= 7);
}
