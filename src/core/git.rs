use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

pub fn get_head_commit(workspace: &Path) -> Result<String> {
    run_git(workspace, &["rev-parse", "HEAD"])
}

pub fn is_repository(workspace: &Path) -> bool {
    run_git(workspace, &["rev-parse", "--is-inside-work-tree"])
        .map(|out| out.trim() == "true")
        .unwrap_or(false)
}

pub fn ensure_repository(workspace: &Path) -> Result<String> {
    let had_repo = is_repository(workspace);
    if !had_repo {
        run_git(workspace, &["init"])?;
    }

    match get_head_commit(workspace) {
        Ok(commit) => Ok(commit),
        Err(_) => {
            ensure_orchestrator_ignored(workspace)?;
            run_git(workspace, &["add", "-A"])?;
            let status = run_git(workspace, &["status", "--porcelain"])?;
            if status.trim().is_empty() {
                run_git(
                    workspace,
                    &[
                        "-c",
                        "user.email=ai-orchestrator@example.local",
                        "-c",
                        "user.name=AI Orchestrator",
                        "commit",
                        "--allow-empty",
                        "-m",
                        "Initial ai-orchestrator baseline",
                    ],
                )?;
            } else {
                run_git(
                    workspace,
                    &[
                        "-c",
                        "user.email=ai-orchestrator@example.local",
                        "-c",
                        "user.name=AI Orchestrator",
                        "commit",
                        "-m",
                        "Initial ai-orchestrator baseline",
                    ],
                )?;
            }
            get_head_commit(workspace)
        }
    }
}

pub fn is_clean_tree(workspace: &Path) -> Result<bool> {
    Ok(status_porcelain(workspace)?.trim().is_empty())
}

pub fn status_porcelain(workspace: &Path) -> Result<String> {
    run_git(workspace, &["status", "--porcelain"])
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

fn ensure_orchestrator_ignored(workspace: &Path) -> Result<()> {
    let entry = ".ai-orchestrator/";
    let gitignore_path = workspace.join(".gitignore");
    let content = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
    if content.lines().any(|line| line.trim() == entry) {
        return Ok(());
    }

    let mut next = content;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(entry);
    next.push('\n');
    std::fs::write(gitignore_path, next)?;
    Ok(())
}

fn path_as_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))
}
