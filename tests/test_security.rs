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

#[test]
fn env_dot_production_is_blocked() {
    let diff = "--- a/.env.production\n+++ b/.env.production\n@@\n+DB_PASS=prod_pass\n";
    let violations = scan_diff(diff);
    assert!(
        violations.iter().any(|v| v.pattern == ".env"),
        "should block .env.production header"
    );
}

#[test]
fn env_dot_local_is_blocked() {
    let diff = "--- a/.env.local\n+++ b/.env.local\n@@\n+API_KEY=localkey\n";
    let violations = scan_diff(diff);
    assert!(
        violations.iter().any(|v| v.pattern == ".env"),
        "should block .env.local header"
    );
}

#[test]
fn multiple_violations_are_all_returned() {
    let diff = concat!(
        "--- a/src/config.rs\n+++ b/src/config.rs\n@@\n",
        "+const API_KEY: &str = \"k1\";\n",
        "+const PASSWORD: &str = \"p1\";\n",
        "+const TOKEN: &str = \"t1\";\n"
    );
    let violations = scan_diff(diff);
    assert!(
        violations.len() >= 3,
        "expected at least 3 violations, got {}",
        violations.len()
    );
}

#[test]
fn violation_reports_correct_line_number() {
    // Linha 1: diff header (não conta violação)
    // Linha 2: diff header (não conta violação)
    // Linha 3: @@ header
    // Linha 4: +const API_KEY ... → violação na linha 4
    let diff = "--- a/src/config.rs\n+++ b/src/config.rs\n@@\n+const API_KEY: &str = \"key\";\n";
    let violations = scan_diff(diff);
    assert!(!violations.is_empty());
    let api_key_violation = violations.iter().find(|v| v.pattern == "API_KEY").unwrap();
    assert_eq!(api_key_violation.line_number, 4);
}
