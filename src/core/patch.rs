use std::collections::HashSet;

use anyhow::{anyhow, Result};

use crate::core::compute_sha256;

#[derive(Debug, Clone)]
pub struct ParsedDiff {
    pub files_modified: Vec<String>,
    pub is_valid_unified_diff: bool,
    pub raw: String,
}

pub fn parse_diff(content: &str) -> Result<ParsedDiff> {
    if content.trim().is_empty() {
        return Err(anyhow!("diff content is empty"));
    }

    let has_old = content.lines().any(|line| line.starts_with("--- "));
    let has_new = content.lines().any(|line| line.starts_with("+++ "));

    if !has_old || !has_new {
        return Err(anyhow!("diff is not a valid unified diff"));
    }

    let mut seen = HashSet::new();
    let mut files_modified = Vec::new();

    for line in content.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            let path = path.trim();
            if !path.is_empty() && seen.insert(path.to_string()) {
                files_modified.push(path.to_string());
            }
        }
    }

    Ok(ParsedDiff {
        files_modified,
        is_valid_unified_diff: true,
        raw: content.to_string(),
    })
}

pub fn compute_patch_hash(content: &str) -> String {
    compute_sha256(content)
}
