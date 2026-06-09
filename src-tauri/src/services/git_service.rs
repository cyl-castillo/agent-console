use crate::services::proc;
use std::path::{Path, PathBuf};

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
pub struct GitCommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author: String,
    pub date_ms: i64,
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
    let inside = proc::command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo)
        .output()?;
    if !inside.status.success() {
        return Ok(GitStatus {
            is_repo: false,
            branch: None,
            changes: Vec::new(),
        });
    }

    let branch_out = proc::command("git")
        .args(["branch", "--show-current"])
        .current_dir(repo)
        .output()?;
    let branch = if branch_out.status.success() {
        let s = String::from_utf8_lossy(&branch_out.stdout)
            .trim()
            .to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    };

    let status_out = proc::command("git")
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
        if line.len() < 3 {
            continue;
        }
        let code = line[..2].to_string();
        let path = line[3..].to_string();
        let staged = !code.starts_with(' ') && !code.starts_with('?');
        let unstaged = !code[1..].starts_with(' ') && !code.starts_with('?');
        let untracked = code == "??";
        changes.push(GitFileChange {
            path,
            code,
            staged,
            unstaged,
            untracked,
        });
    }

    Ok(GitStatus {
        is_repo: true,
        branch,
        changes,
    })
}

/// Resolve a frontend-supplied path strictly inside the repo. Rejects absolute
/// paths and anything that escapes via `..` or symlinks (canonicalize resolves
/// both, so the target must exist — which holds for the read/delete callers).
fn resolve_in_repo(repo: &Path, file: &str) -> AppResult<PathBuf> {
    if Path::new(file).is_absolute() {
        return Err(AppError::InvalidArgument(format!(
            "path escapes repo: {file}"
        )));
    }
    let repo_canon = repo.canonicalize()?;
    let canon = repo_canon.join(file).canonicalize()?;
    if !canon.starts_with(&repo_canon) {
        return Err(AppError::InvalidArgument(format!(
            "path escapes repo: {file}"
        )));
    }
    Ok(canon)
}

/// Unified diff for a single file. Falls back to a synthetic diff for
/// untracked files (whose content is not yet tracked by git).
pub fn diff_file(repo: &Path, file: &str) -> AppResult<String> {
    // Untracked? Show full file content as additions.
    let ls = proc::command("git")
        .args(["ls-files", "--error-unmatch", "--", file])
        .current_dir(repo)
        .output()?;
    if !ls.status.success() {
        let Ok(abs) = resolve_in_repo(repo, file) else {
            return Ok(String::new());
        };
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
    let out = proc::command("git")
        .args(["diff", "--no-color", "HEAD", "--", file])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(AppError::Other(format!("git diff: {msg}")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// `git add -- file`. Works for modifications, additions, deletions, and untracked files.
pub fn stage_file(repo: &Path, file: &str) -> AppResult<()> {
    let out = proc::command("git")
        .args(["add", "--", file])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(AppError::Other(format!("git add: {msg}")));
    }
    Ok(())
}

/// `git restore --staged -- file`. Removes from index, leaves working tree alone.
/// Falls back to `git reset HEAD -- file` for older git versions or for files
/// staged in an initial commit (no HEAD yet).
pub fn unstage_file(repo: &Path, file: &str) -> AppResult<()> {
    let out = proc::command("git")
        .args(["restore", "--staged", "--", file])
        .current_dir(repo)
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    // Initial commit / no HEAD yet: `git rm --cached` removes from index without touching disk.
    let fallback = proc::command("git")
        .args(["rm", "--cached", "--quiet", "--", file])
        .current_dir(repo)
        .output()?;
    if !fallback.status.success() {
        let msg = String::from_utf8_lossy(&fallback.stderr).to_string();
        return Err(AppError::Other(format!("git unstage: {msg}")));
    }
    Ok(())
}

/// `git commit -m <message>`. Returns the new commit SHA.
pub fn commit(repo: &Path, message: &str) -> AppResult<String> {
    if message.trim().is_empty() {
        return Err(AppError::InvalidArgument("commit message is empty".into()));
    }
    let out = proc::command("git")
        .args(["commit", "-m", message])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        return Err(AppError::Other(format!("git commit failed: {msg}{stdout}")));
    }
    let sha_out = proc::command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()?;
    let sha = String::from_utf8_lossy(&sha_out.stdout).trim().to_string();
    Ok(sha)
}

/// `git log -n <limit> --pretty=... -- <file>`. Empty if file is untracked or
/// has no history yet. Best-effort: returns empty list on failure.
pub fn file_log(repo: &Path, file: &str, limit: u32) -> AppResult<Vec<GitCommitInfo>> {
    // Separator unlikely to appear in subjects/author names.
    const SEP: &str = "\u{1f}";
    let format = format!("%H{SEP}%h{SEP}%s{SEP}%an{SEP}%at");
    let out = proc::command("git")
        .args([
            "log",
            &format!("-n{limit}"),
            &format!("--pretty=format:{format}"),
            "--",
            file,
        ])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let mut commits = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.split(SEP).collect();
        if parts.len() != 5 {
            continue;
        }
        let date_ms = parts[4].parse::<i64>().unwrap_or(0) * 1000;
        commits.push(GitCommitInfo {
            sha: parts[0].to_string(),
            short_sha: parts[1].to_string(),
            subject: parts[2].to_string(),
            author: parts[3].to_string(),
            date_ms,
        });
    }
    Ok(commits)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub last_commit_ms: i64,
    pub last_subject: String,
}

/// List local branches with ahead/behind vs their upstream (if any) and the
/// latest commit info. Output is sorted by recency desc.
pub fn branches(repo: &Path) -> AppResult<Vec<BranchInfo>> {
    const SEP: &str = "\u{1f}";
    // %(HEAD) yields "*" for the current branch, " " otherwise.
    // %(upstream:short) may be empty when there is no tracking branch.
    let format = format!(
        "%(HEAD){SEP}%(refname:short){SEP}%(upstream:short){SEP}%(committerdate:unix){SEP}%(contents:subject)"
    );
    let out = proc::command("git")
        .args([
            "for-each-ref",
            "--sort=-committerdate",
            &format!("--format={format}"),
            "refs/heads",
        ])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(AppError::Other(format!("git for-each-ref: {msg}")));
    }
    let raw = String::from_utf8_lossy(&out.stdout).to_string();
    let mut result = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.splitn(5, SEP).collect();
        if parts.len() < 5 {
            continue;
        }
        let current = parts[0].trim() == "*";
        let name = parts[1].to_string();
        let upstream = if parts[2].is_empty() {
            None
        } else {
            Some(parts[2].to_string())
        };
        let last_commit_ms = parts[3].parse::<i64>().unwrap_or(0) * 1000;
        let last_subject = parts[4].to_string();

        let (ahead, behind) = if let Some(up) = upstream.as_ref() {
            ahead_behind(repo, &name, up).unwrap_or((0, 0))
        } else {
            (0, 0)
        };

        result.push(BranchInfo {
            name,
            current,
            upstream,
            ahead,
            behind,
            last_commit_ms,
            last_subject,
        });
    }
    Ok(result)
}

fn ahead_behind(repo: &Path, branch: &str, upstream: &str) -> AppResult<(u32, u32)> {
    let out = proc::command("git")
        .args([
            "rev-list",
            "--left-right",
            "--count",
            &format!("{upstream}...{branch}"),
        ])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Ok((0, 0));
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let nums: Vec<u32> = raw
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() != 2 {
        return Ok((0, 0));
    }
    // Left side = upstream (behind), right side = branch (ahead).
    Ok((nums[1], nums[0]))
}

/// `git checkout <name>`. Fails loudly if the working tree has conflicting
/// uncommitted changes — that's git's natural protection.
pub fn checkout_branch(repo: &Path, name: &str) -> AppResult<()> {
    let out = proc::command("git")
        .args(["checkout", name])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(AppError::Other(format!("git checkout: {msg}")));
    }
    Ok(())
}

/// Recent commit messages from the current branch (subject + body).
/// Best-effort: returns empty on failure or detached HEAD.
pub fn recent_messages(repo: &Path, limit: u32) -> AppResult<Vec<String>> {
    const SEP: &str = "\u{1e}";
    let format = format!("%B{SEP}");
    let out = proc::command("git")
        .args([
            "log",
            &format!("-n{limit}"),
            &format!("--pretty=format:{format}"),
        ])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    let raw = String::from_utf8_lossy(&out.stdout).to_string();
    let msgs: Vec<String> = raw
        .split(SEP)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(msgs)
}

/// Full commit message (subject + body) of HEAD. Empty string if there is no
/// HEAD yet (fresh repo).
pub fn head_message(repo: &Path) -> AppResult<String> {
    let out = proc::command("git")
        .args(["log", "-1", "--pretty=%B"])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// `git commit --amend -m <message>`. Allows amending HEAD with no new
/// staged changes (just rewords the message).
pub fn amend_commit(repo: &Path, message: &str) -> AppResult<String> {
    if message.trim().is_empty() {
        return Err(AppError::InvalidArgument("commit message is empty".into()));
    }
    let out = proc::command("git")
        .args(["commit", "--amend", "-m", message])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        return Err(AppError::Other(format!(
            "git commit --amend failed: {msg}{stdout}"
        )));
    }
    let sha_out = proc::command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()?;
    Ok(String::from_utf8_lossy(&sha_out.stdout).trim().to_string())
}

/// Revert a single change. Tracked files: `git checkout HEAD -- file`.
/// Untracked files: delete from disk.
pub fn revert_file(repo: &Path, file: &str) -> AppResult<()> {
    let ls = proc::command("git")
        .args(["ls-files", "--error-unmatch", "--", file])
        .current_dir(repo)
        .output()?;

    if ls.status.success() {
        let out = proc::command("git")
            .args(["checkout", "HEAD", "--", file])
            .current_dir(repo)
            .output()?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr).to_string();
            return Err(AppError::Other(format!("git checkout: {msg}")));
        }
        return Ok(());
    }

    // Untracked → remove from working tree, but only if the path resolves
    // inside the repo. A missing file is a no-op (already gone).
    let abs = match resolve_in_repo(repo, file) {
        Ok(p) => p,
        Err(AppError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if abs.is_file() {
        std::fs::remove_file(&abs)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_in_repo_accepts_inside_rejects_escapes() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo = std::env::temp_dir().join(format!("ac-resolve-{nanos}"));
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::write(repo.join("inside.txt"), "x").unwrap();
        std::fs::write(repo.join("sub/nested.txt"), "x").unwrap();
        let outside = std::env::temp_dir().join(format!("ac-resolve-outside-{nanos}.txt"));
        std::fs::write(&outside, "y").unwrap();

        assert!(resolve_in_repo(&repo, "inside.txt").is_ok());
        assert!(resolve_in_repo(&repo, "sub/nested.txt").is_ok());
        // `..` that stays inside resolves fine.
        assert!(resolve_in_repo(&repo, "sub/../inside.txt").is_ok());

        // Absolute paths are rejected outright.
        assert!(matches!(
            resolve_in_repo(&repo, outside.to_str().unwrap()),
            Err(AppError::InvalidArgument(_))
        ));
        // Traversal to an existing file outside the repo is rejected.
        let name = outside.file_name().unwrap().to_string_lossy().to_string();
        assert!(matches!(
            resolve_in_repo(&repo, &format!("../{name}")),
            Err(AppError::InvalidArgument(_))
        ));
        // Nonexistent paths surface as io errors (callers treat as no-op).
        assert!(matches!(
            resolve_in_repo(&repo, "no-such-file.txt"),
            Err(AppError::Io(_))
        ));

        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_file(&outside);
    }
}
