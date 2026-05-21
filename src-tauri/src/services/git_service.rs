use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub changes: Vec<GitFileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileChange {
    pub path: String,
    /// Two-char porcelain code, e.g. " M", "M ", "??", "A ", "MM".
    pub code: String,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
}

/// `git status --porcelain=v1 -uall` + `git branch --show-current`.
/// Returns `is_repo = false` when the directory is not a git repo
/// (no error — that's a normal state).
pub fn status(repo: &Path) -> AppResult<GitStatus> {
    if !repo.exists() {
        return Err(AppError::NotFound(repo.display().to_string()));
    }

    // Cheap repo detection.
    let inside = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo)
        .output()?;
    if !inside.status.success() {
        return Ok(GitStatus { is_repo: false, branch: None, changes: Vec::new() });
    }

    let branch_out = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo)
        .output()?;
    let branch = if branch_out.status.success() {
        let s = String::from_utf8_lossy(&branch_out.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    } else {
        None
    };

    let status_out = Command::new("git")
        .args(["status", "--porcelain=v1", "-uall", "--no-renames"])
        .current_dir(repo)
        .output()?;
    if !status_out.status.success() {
        let msg = String::from_utf8_lossy(&status_out.stderr).to_string();
        return Err(AppError::Other(format!("git status: {msg}")));
    }

    let raw = String::from_utf8_lossy(&status_out.stdout);
    let mut changes = Vec::new();
    for line in raw.lines() {
        if line.len() < 3 { continue; }
        let code = line[..2].to_string();
        let path = line[3..].to_string();
        let staged = !code.starts_with(' ') && !code.starts_with('?');
        let unstaged = !code[1..].starts_with(' ') && !code.starts_with('?');
        let untracked = code == "??";
        changes.push(GitFileChange { path, code, staged, unstaged, untracked });
    }

    Ok(GitStatus { is_repo: true, branch, changes })
}

/// Unified diff for a single file. Falls back to a synthetic diff for
/// untracked files (whose content is not yet tracked by git).
pub fn diff_file(repo: &Path, file: &str) -> AppResult<String> {
    // Untracked? Show full file content as additions.
    let ls = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", file])
        .current_dir(repo)
        .output()?;
    if !ls.status.success() {
        let abs = repo.join(file);
        if let Ok(content) = std::fs::read_to_string(&abs) {
            let mut out = format!("diff --git a/{file} b/{file}\n");
            out.push_str("new file (untracked)\n");
            out.push_str(&format!("--- /dev/null\n+++ b/{file}\n"));
            for line in content.lines() {
                out.push('+');
                out.push_str(line);
                out.push('\n');
            }
            return Ok(out);
        }
        return Ok(String::new());
    }

    // Tracked: combine staged + unstaged so we show the full delta vs HEAD.
    let out = Command::new("git")
        .args(["diff", "--no-color", "HEAD", "--", file])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(AppError::Other(format!("git diff: {msg}")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Revert a single change. Tracked files: `git checkout HEAD -- file`.
/// Untracked files: delete from disk.
pub fn revert_file(repo: &Path, file: &str) -> AppResult<()> {
    let ls = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", file])
        .current_dir(repo)
        .output()?;

    if ls.status.success() {
        let out = Command::new("git")
            .args(["checkout", "HEAD", "--", file])
            .current_dir(repo)
            .output()?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr).to_string();
            return Err(AppError::Other(format!("git checkout: {msg}")));
        }
        return Ok(());
    }

    // Untracked → remove from working tree.
    let abs = repo.join(file);
    if abs.is_file() {
        std::fs::remove_file(&abs)?;
    }
    Ok(())
}
