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

#[test]
fn patch_hash_differs_for_different_diffs() {
    let d1 = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let d2 = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+other\n";
    assert_ne!(compute_patch_hash(d1), compute_patch_hash(d2));
}

#[test]
fn new_file_diff_from_dev_null_is_parsed() {
    let diff = "--- /dev/null\n+++ b/src/new_file.rs\n@@ -0,0 +1 @@\n+fn brand_new() {}\n";
    let parsed = parse_diff(diff).expect("new-file diff should parse");
    assert!(parsed.is_valid_unified_diff);
    assert_eq!(parsed.files_modified, vec!["src/new_file.rs"]);
}

#[test]
fn multi_hunk_diff_in_same_file_extracts_single_path() {
    let diff = concat!(
        "--- a/src/lib.rs\n+++ b/src/lib.rs\n",
        "@@ -1 +1 @@\n-fn old_a() {}\n+fn new_a() {}\n",
        "@@ -10 +10 @@\n-fn old_b() {}\n+fn new_b() {}\n"
    );
    let parsed = parse_diff(diff).expect("multi-hunk diff should parse");
    assert_eq!(parsed.files_modified.len(), 1);
    assert_eq!(parsed.files_modified[0], "src/lib.rs");
}

#[test]
fn diff_with_multiple_files_deduplicates_paths() {
    // Mesmo arquivo aparece duas vezes — não deve duplicar
    let diff = concat!(
        "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n",
        "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -5 +5 @@\n-old2\n+new2\n"
    );
    let parsed = parse_diff(diff).expect("duplicate-file diff should parse");
    assert_eq!(parsed.files_modified.len(), 1);
}
