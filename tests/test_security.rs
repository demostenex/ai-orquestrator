use ai_orchestrator::core::security::scan_diff;

#[test]
fn clean_diff_passes() {
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@\n-pub fn old() {}\n+pub fn new() {}\n";
    assert!(scan_diff(diff).is_empty());
}

#[test]
fn api_key_is_blocked() {
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@\n+const API_KEY: &str = \"secret\";\n";
    assert!(scan_diff(diff).iter().any(|v| v.pattern == "API_KEY"));
}

#[test]
fn env_header_is_blocked() {
    let diff = "--- a/.env\n+++ b/.env\n@@\n+TOKEN=abc\n";
    let violations = scan_diff(diff);
    assert!(violations.iter().any(|v| v.pattern == ".env"));
}

#[test]
fn token_is_blocked() {
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@\n+let TOKEN = \"abc\";\n";
    assert!(scan_diff(diff).iter().any(|v| v.pattern == "TOKEN"));
}

#[test]
fn password_is_blocked() {
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@\n+let PASSWORD = \"abc\";\n";
    assert!(scan_diff(diff).iter().any(|v| v.pattern == "PASSWORD"));
}

#[test]
fn lowercase_variants_are_blocked() {
    let diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@\n+let api_key = \"abc\";\n+let token = \"abc\";\n+let password = \"abc\";\n+let secret = \"abc\";\n+let private_key = \"abc\";\n";
    let violations = scan_diff(diff);
    assert!(violations.iter().any(|v| v.pattern == "api_key"));
    assert!(violations.iter().any(|v| v.pattern == "token"));
    assert!(violations.iter().any(|v| v.pattern == "password"));
    assert!(violations.iter().any(|v| v.pattern == "secret"));
    assert!(violations.iter().any(|v| v.pattern == "private_key"));
}
