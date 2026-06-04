use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityViolation {
    pub pattern: String,
    pub line_number: usize,
}

pub fn scan_diff(content: &str) -> Vec<SecurityViolation> {
    let detectors = [
        (Regex::new(r"\bAPI_KEY\b").expect("valid regex"), "API_KEY"),
        (Regex::new(r"\bapi_key\b").expect("valid regex"), "api_key"),
        (Regex::new(r"\bTOKEN\b").expect("valid regex"), "TOKEN"),
        (Regex::new(r"\btoken\b").expect("valid regex"), "token"),
        (
            Regex::new(r"\bPASSWORD\b").expect("valid regex"),
            "PASSWORD",
        ),
        (
            Regex::new(r"\bpassword\b").expect("valid regex"),
            "password",
        ),
        (Regex::new(r"\bSECRET\b").expect("valid regex"), "SECRET"),
        (Regex::new(r"\bsecret\b").expect("valid regex"), "secret"),
        (
            Regex::new(r"\bPRIVATE_KEY\b").expect("valid regex"),
            "PRIVATE_KEY",
        ),
        (
            Regex::new(r"\bprivate_key\b").expect("valid regex"),
            "private_key",
        ),
    ];

    let mut violations = Vec::new();

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        for (regex, label) in &detectors {
            if regex.is_match(line) {
                violations.push(SecurityViolation {
                    pattern: (*label).to_string(),
                    line_number,
                });
            }
        }

        if (line.starts_with("--- ") || line.starts_with("+++ ")) && contains_env_path(line) {
            violations.push(SecurityViolation {
                pattern: ".env".to_string(),
                line_number,
            });
        }
    }

    violations
}

fn contains_env_path(line: &str) -> bool {
    let candidate = line.split_whitespace().nth(1).unwrap_or_default();
    let normalized = candidate.trim_start_matches("a/").trim_start_matches("b/");

    normalized == ".env" || normalized.starts_with(".env.")
}
