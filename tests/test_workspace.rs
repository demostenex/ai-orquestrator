mod common;

use std::fs;

use ai_orchestrator::agents::dev::build_workspace_snapshot;
use tempfile::TempDir;

// ── Inclusão de arquivos ──────────────────────────────────────────────────────

#[test]
fn snapshot_includes_rust_files() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lib.rs"), "fn main() {}").unwrap();
    fs::write(src.join("mod.rs"), "pub mod lib;").unwrap();

    let snapshot = build_workspace_snapshot(dir.path());
    assert!(
        snapshot.contains("lib.rs"),
        "snapshot should include lib.rs"
    );
    assert!(
        snapshot.contains("fn main()"),
        "snapshot should include file content"
    );
    assert!(
        snapshot.contains("mod.rs"),
        "snapshot should include mod.rs"
    );
}

// ── Exclusão de diretórios ignorados ─────────────────────────────────────────

#[test]
fn snapshot_excludes_git_dir() {
    let dir = TempDir::new().unwrap();
    let git_dir = dir.path().join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("config"), "[core]\nbare = false").unwrap();

    let snapshot = build_workspace_snapshot(dir.path());
    assert!(
        !snapshot.contains(".git/config"),
        "snapshot should not include .git contents"
    );
}

#[test]
fn snapshot_excludes_target_dir() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("target").join("debug");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("binary"), "ELF binary content").unwrap();

    // Arquivo legítimo fora de target
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

    let snapshot = build_workspace_snapshot(dir.path());
    assert!(
        !snapshot.contains("binary"),
        "snapshot should not include target/ contents"
    );
    assert!(
        snapshot.contains("Cargo.toml"),
        "snapshot should include Cargo.toml"
    );
}

#[test]
fn snapshot_excludes_node_modules() {
    let dir = TempDir::new().unwrap();
    let nm = dir.path().join("node_modules").join("some-package");
    fs::create_dir_all(&nm).unwrap();
    fs::write(nm.join("index.js"), "module.exports = {}").unwrap();

    fs::write(dir.path().join("package.json"), "{\"name\": \"test\"}").unwrap();

    let snapshot = build_workspace_snapshot(dir.path());
    assert!(
        !snapshot.contains("node_modules"),
        "snapshot should not include node_modules"
    );
    assert!(
        snapshot.contains("package.json"),
        "snapshot should include package.json"
    );
}

// ── Workspace vazio ───────────────────────────────────────────────────────────

#[test]
fn snapshot_returns_default_message_on_empty_workspace() {
    let dir = TempDir::new().unwrap();
    let snapshot = build_workspace_snapshot(dir.path());
    assert!(
        snapshot.contains("vazio") || snapshot.contains("nenhum"),
        "empty workspace should return descriptive message, got: {snapshot}"
    );
}

// ── Exclusão de arquivos grandes ──────────────────────────────────────────────

#[test]
fn snapshot_excludes_large_files() {
    let dir = TempDir::new().unwrap();
    // Cria arquivo > 64KB (limite é 64 * 1024 = 65536 bytes)
    let large_content = "x".repeat(66_000);
    fs::write(dir.path().join("large.rs"), &large_content).unwrap();
    fs::write(dir.path().join("small.rs"), "fn small() {}").unwrap();

    let snapshot = build_workspace_snapshot(dir.path());
    assert!(
        !snapshot.contains("large.rs"),
        "large file should be excluded"
    );
    assert!(
        snapshot.contains("small.rs"),
        "small file should be included"
    );
}
