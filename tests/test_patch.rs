use ai_orchestrator::core::patch::{compute_patch_hash, parse_diff};

#[test]
fn valid_unified_diff_is_parsed() {
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let parsed = parse_diff(diff).expect("diff should parse");
    assert!(parsed.is_valid_unified_diff);
    assert_eq!(parsed.raw, diff);
}

#[test]
fn file_paths_are_extracted_from_plus_plus_plus_lines() {
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let parsed = parse_diff(diff).expect("diff should parse");
    assert_eq!(parsed.files_modified, vec!["src/lib.rs", "src/main.rs"]);
}

#[test]
fn invalid_diff_without_headers_fails() {
    let err = parse_diff("@@ -1 +1 @@\n-old\n+new\n").expect_err("diff should fail");
    assert!(err.to_string().contains("unified diff"));
}

#[test]
fn empty_diff_fails() {
    let err = parse_diff("   ").expect_err("empty diff should fail");
    assert!(err.to_string().contains("empty"));
}

#[test]
fn patch_hash_is_deterministic() {
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
    assert_eq!(compute_patch_hash(diff), compute_patch_hash(diff));
}
