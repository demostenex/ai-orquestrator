pub mod cli_runner;
pub mod config;
pub mod db;
pub mod git;
pub mod handoff;
pub mod memory;
pub mod patch;
pub mod security;
pub mod session;
pub mod stream;

use sha2::{Digest, Sha256};

pub fn strip_json_fences(s: &str) -> &str {
    let trimmed = s.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim();

    trimmed.strip_suffix("```").unwrap_or(trimmed).trim()
}

pub fn compute_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}
