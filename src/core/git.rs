use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

pub fn get_head_commit(workspace: &Path) -> Result<String> {
    run_git(workspace, &["rev-parse", "HEAD"])
}

pub fn is_clean_tree(workspace: &Path) -> Result<bool> {
    Ok(run_git(workspace, &["status", "--porcelain"])?
        .trim()
        .is_empty())
}

pub fn apply_check(workspace: &Path, patch_path: &Path) -> Result<()> {
    run_git(workspace, &["apply", "--check", path_as_str(patch_path)?]).map(|_| ())
}

pub fn apply_patch(workspace: &Path, patch_path: &Path) -> Result<()> {
    run_git(workspace, &["apply", path_as_str(patch_path)?]).map(|_| ())
}

pub fn add_all(workspace: &Path) -> Result<()> {
    run_git(workspace, &["add", "-A"]).map(|_| ())
}

pub fn commit(workspace: &Path, message: &str) -> Result<String> {
    run_git(workspace, &["commit", "-m", message])
}

pub fn current_branch(workspace: &Path) -> Result<String> {
    run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"])
}

pub fn last_commit_summary(workspace: &Path) -> Result<String> {
    run_git(workspace, &["log", "-1", "--pretty=%h %s"])
}

fn run_git(workspace: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("--no-pager")
        .args(args)
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to run git {:?} in {}", args, workspace.display()))?;

    if !output.status.success() {
        return Err(anyhow!(
            "git {:?} failed in {}: {}",
            args,
            workspace.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn path_as_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))
}
