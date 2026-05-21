use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// A point-in-time capture of the working tree, stored as a non-HEAD git commit
/// kept alive via `refs/agent-console/<id>`. Includes untracked files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub commit_sha: String,
}

/// Build a snapshot commit without touching HEAD or the user's index.
/// Strategy: write the full working tree into a temporary index, write-tree → commit-tree → update-ref.
/// Returns `None` if the project is not a git repo (no snapshot to create).
pub fn create(repo: &Path, id: &str) -> AppResult<Option<Snapshot>> {
    if !is_git_repo(repo)? {
        return Ok(None);
    }

    let tmp_idx = repo.join(".git").join(format!("agent-console-idx-{id}"));
    let tmp_idx_str = tmp_idx.to_string_lossy().to_string();

    // Try seed the temp index from HEAD; tolerate empty repos.
    let _ = Command::new("git")
        .env("GIT_INDEX_FILE", &tmp_idx_str)
        .args(["read-tree", "HEAD"])
        .current_dir(repo)
        .output();

    // Stage everything from working tree (honors .gitignore).
    let add = Command::new("git")
        .env("GIT_INDEX_FILE", &tmp_idx_str)
        .args(["add", "-A"])
        .current_dir(repo)
        .output()?;
    if !add.status.success() {
        cleanup(&tmp_idx);
        return Err(AppError::Other(format!(
            "snapshot add: {}", String::from_utf8_lossy(&add.stderr)
        )));
    }

    // Capture tree.
    let tree_out = Command::new("git")
        .env("GIT_INDEX_FILE", &tmp_idx_str)
        .args(["write-tree"])
        .current_dir(repo)
        .output()?;
    cleanup(&tmp_idx);
    if !tree_out.status.success() {
        return Err(AppError::Other(format!(
            "write-tree: {}", String::from_utf8_lossy(&tree_out.stderr)
        )));
    }
    let tree_sha = String::from_utf8_lossy(&tree_out.stdout).trim().to_string();

    // commit-tree with optional parent.
    let mut commit_args = vec!["commit-tree".to_string(), tree_sha.clone()];
    if let Some(head) = head_sha(repo) {
        commit_args.push("-p".to_string());
        commit_args.push(head);
    }
    commit_args.push("-m".to_string());
    commit_args.push(format!("agent-console snapshot {id}"));

    let commit_out = Command::new("git")
        .args(&commit_args)
        .current_dir(repo)
        .output()?;
    if !commit_out.status.success() {
        return Err(AppError::Other(format!(
            "commit-tree: {}", String::from_utf8_lossy(&commit_out.stderr)
        )));
    }
    let commit_sha = String::from_utf8_lossy(&commit_out.stdout).trim().to_string();

    // Pin via ref so GC won't collect it.
    let _ = Command::new("git")
        .args(["update-ref", &snapshot_ref(id), &commit_sha])
        .current_dir(repo)
        .output()?;

    Ok(Some(Snapshot { id: id.to_string(), commit_sha }))
}

/// Force working tree + index to match this snapshot's tree. Doesn't move HEAD.
pub fn restore(repo: &Path, commit_sha: &str) -> AppResult<()> {
    // Resolve the tree from the commit.
    let tree = Command::new("git")
        .args(["rev-parse", &format!("{commit_sha}^{{tree}}")])
        .current_dir(repo)
        .output()?;
    if !tree.status.success() {
        return Err(AppError::Other(format!(
            "snapshot not found: {}", String::from_utf8_lossy(&tree.stderr)
        )));
    }
    let tree_sha = String::from_utf8_lossy(&tree.stdout).trim().to_string();

    let out = Command::new("git")
        .args(["read-tree", "--reset", "-u", &tree_sha])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "read-tree restore: {}", String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

pub fn delete(repo: &Path, id: &str) -> AppResult<()> {
    let _ = Command::new("git")
        .args(["update-ref", "-d", &snapshot_ref(id)])
        .current_dir(repo)
        .output();
    Ok(())
}

fn snapshot_ref(id: &str) -> String {
    format!("refs/agent-console/{id}")
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn is_git_repo(repo: &Path) -> AppResult<bool> {
    let out = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo)
        .output()?;
    Ok(out.status.success())
}

fn head_sha(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
