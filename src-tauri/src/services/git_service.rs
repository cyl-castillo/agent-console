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

/// Paths staged for the next commit (`git diff --cached --name-only`),
/// relative to the repo root.
pub fn staged_files(repo: &Path) -> AppResult<Vec<String>> {
    let out = proc::command("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
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

/// Current HEAD commit sha (full form).
pub fn head_sha(repo: &Path) -> AppResult<String> {
    let out = proc::command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Commit ONLY the given paths (`git commit -m <msg> -- <paths>`), leaving
/// whatever else the user has staged exactly as staged. Untracked paths are
/// added first; the pathspec form then commits their working-tree content
/// without sweeping the rest of the index — the P2 attach flow must never
/// smuggle the user's half-staged work into its packet commit.
pub fn commit_paths(repo: &Path, message: &str, paths: &[String]) -> AppResult<String> {
    if message.trim().is_empty() {
        return Err(AppError::InvalidArgument("commit message is empty".into()));
    }
    if paths.is_empty() {
        return Err(AppError::InvalidArgument("no paths to commit".into()));
    }
    let mut add = vec!["add", "--"];
    add.extend(paths.iter().map(String::as_str));
    let out = git(repo, &add)?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let mut args = vec!["commit", "-m", message, "--"];
    args.extend(paths.iter().map(String::as_str));
    let out = git(repo, &args)?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "git commit failed: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        )));
    }
    head_sha(repo)
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn git(args: &[&str], cwd: &Path) -> std::process::Output {
        proc::command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap()
    }

    fn init_repo(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo = std::env::temp_dir().join(format!("ac-git-{tag}-{nanos}"));
        fs::create_dir_all(&repo).unwrap();
        git(&["init", "-q"], &repo);
        git(&["config", "user.email", "t@t"], &repo);
        git(&["config", "user.name", "T"], &repo);
        git(&["config", "commit.gpgsign", "false"], &repo);
        repo
    }

    fn change_for<'a>(st: &'a GitStatus, path: &str) -> Option<&'a GitFileChange> {
        st.changes.iter().find(|c| c.path == path)
    }

    /// P2 attach flow: the packet commit must carry ONLY its own paths —
    /// whatever the user had staged stays staged, untouched.
    #[test]
    fn commit_paths_leaves_the_rest_of_the_index_alone() {
        let repo = init_repo("cpaths");
        fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "-A"], &repo);
        git(&["commit", "-qm", "seed"], &repo);

        // The user half-staged their own work…
        fs::write(repo.join("a.txt"), "two\n").unwrap();
        git(&["add", "a.txt"], &repo);
        // …and the attach flow drops an untracked packet.
        fs::create_dir_all(repo.join(".testigo/proofs")).unwrap();
        fs::write(repo.join(".testigo/proofs/p.proofpack.json"), "{}\n").unwrap();

        let sha = commit_paths(
            &repo,
            "Attach proof packet (t)",
            &[".testigo/proofs/p.proofpack.json".into()],
        )
        .unwrap();
        assert_eq!(sha, head_sha(&repo).unwrap());

        // The packet is committed; a.txt is NOT in that commit and stays staged.
        let shown = git(&["show", "--name-only", "--format=", "HEAD"], &repo);
        let files = String::from_utf8_lossy(&shown.stdout).to_string();
        assert!(files.contains(".testigo/proofs/p.proofpack.json"));
        assert!(
            !files.contains("a.txt"),
            "user's staged work must not be swept: {files}"
        );
        let st = status(&repo).unwrap();
        let a = change_for(&st, "a.txt").expect("a.txt still pending");
        assert!(a.staged, "a.txt stays staged after the packet commit");
    }

    #[test]
    fn non_repo_status_is_a_normal_state() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let plain = std::env::temp_dir().join(format!("ac-git-norepo-{nanos}"));
        fs::create_dir_all(&plain).unwrap();
        let st = status(&plain).unwrap();
        assert!(!st.is_repo, "non-repo is not an error, just is_repo=false");
        assert!(st.changes.is_empty());
        let _ = fs::remove_dir_all(&plain);
    }

    #[test]
    fn stage_unstage_commit_diff_revert_lifecycle() {
        let repo = init_repo("ops");
        fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "-A"], &repo);
        git(&["commit", "-qm", "seed"], &repo);

        // Clean tree.
        let st = status(&repo).unwrap();
        assert!(st.is_repo);
        assert!(st.changes.is_empty());

        // Untracked file: ?? code, synthetic diff shows full content as adds.
        fs::write(repo.join("b.txt"), "hello\n").unwrap();
        let st = status(&repo).unwrap();
        let b = change_for(&st, "b.txt").expect("b.txt listed");
        assert!(b.untracked);
        assert_eq!(b.code, "??");
        let d = diff_file(&repo, "b.txt").unwrap();
        assert!(d.contains("new file (untracked)"));
        assert!(d.contains("+hello"));

        // Stage → A , unstage → back to untracked.
        stage_file(&repo, "b.txt").unwrap();
        let st = status(&repo).unwrap();
        let b = change_for(&st, "b.txt").unwrap();
        assert!(b.staged && !b.untracked);
        unstage_file(&repo, "b.txt").unwrap();
        let st = status(&repo).unwrap();
        assert!(change_for(&st, "b.txt").unwrap().untracked);

        // Commit clears the change list and head_message reflects it.
        stage_file(&repo, "b.txt").unwrap();
        let sha = commit(&repo, "add b").unwrap();
        assert_eq!(sha.len(), 40, "commit returns the full sha");
        assert!(status(&repo).unwrap().changes.is_empty());
        assert_eq!(head_message(&repo).unwrap(), "add b");
        let msgs = recent_messages(&repo, 5).unwrap();
        assert_eq!(msgs.first().map(String::as_str), Some("add b"));

        // Tracked modification: diff vs HEAD shows old and new lines.
        fs::write(repo.join("b.txt"), "world\n").unwrap();
        let st = status(&repo).unwrap();
        assert!(change_for(&st, "b.txt").unwrap().unstaged);
        let d = diff_file(&repo, "b.txt").unwrap();
        assert!(d.contains("-hello"));
        assert!(d.contains("+world"));

        // Revert tracked → content restored from HEAD.
        revert_file(&repo, "b.txt").unwrap();
        assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "hello\n");
        assert!(status(&repo).unwrap().changes.is_empty());

        // Revert untracked → file deleted; missing file is a no-op.
        fs::write(repo.join("c.txt"), "tmp").unwrap();
        revert_file(&repo, "c.txt").unwrap();
        assert!(!repo.join("c.txt").exists());
        revert_file(&repo, "c.txt").unwrap();

        // Amend rewrites the message without adding a commit.
        let amended = amend_commit(&repo, "add b (amended)").unwrap();
        assert_ne!(amended, sha, "amend creates a new sha");
        assert_eq!(head_message(&repo).unwrap(), "add b (amended)");
        let count = git(&["rev-list", "--count", "HEAD"], &repo);
        assert_eq!(String::from_utf8_lossy(&count.stdout).trim(), "2");

        // file_log sees both commits that touched b.txt.
        let log = file_log(&repo, "b.txt", 10).unwrap();
        assert_eq!(log.len(), 1, "b.txt was touched by one surviving commit");
        assert_eq!(log[0].subject, "add b (amended)");

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn branch_listing_and_checkout() {
        let repo = init_repo("branch");
        fs::write(repo.join("a.txt"), "x").unwrap();
        git(&["add", "-A"], &repo);
        git(&["commit", "-qm", "seed"], &repo);
        git(&["branch", "feature"], &repo);

        let bs = branches(&repo).unwrap();
        let names: Vec<_> = bs.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"feature"));
        assert_eq!(bs.iter().filter(|b| b.current).count(), 1);

        checkout_branch(&repo, "feature").unwrap();
        let bs = branches(&repo).unwrap();
        let cur = bs.iter().find(|b| b.current).unwrap();
        assert_eq!(cur.name, "feature");

        // Unknown branch is a clear error.
        assert!(checkout_branch(&repo, "nope").is_err());

        let _ = fs::remove_dir_all(&repo);
    }

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

// ---- push + pull request (P1) ---------------------------------------------

/// `git <args>` in `repo`, output captured (same shape as worktree_service's).
fn git(repo: &Path, args: &[&str]) -> AppResult<std::process::Output> {
    Ok(proc::command("git").args(args).current_dir(repo).output()?)
}

/// What `push_current` did, and where the PR lives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub branch: String,
    pub remote: String,
    /// True when this push created the upstream (`git push -u`).
    pub set_upstream: bool,
    /// The remote's default branch (`origin/HEAD`), when known.
    pub default_branch: Option<String>,
    /// "Create PR/MR for this branch" URL for hosts we recognize (GitHub,
    /// GitLab); None on the default branch or an unknown host.
    pub pr_url: Option<String>,
}

/// Push the current branch. No upstream yet ⇒ `git push -u <remote> <branch>`
/// (remote = the branch's, else `origin`); with one ⇒ plain `git push`. The
/// loop the console had ended in `commit`; every other tool in the space
/// ends in a PR — this is the first half of closing that gap, the PR link
/// is the second (`pr_url_for`).
pub fn push_current(repo: &Path) -> AppResult<PushResult> {
    let branch = current_branch(repo)?;
    let upstream = upstream_of(repo, &branch);
    let remote = upstream
        .as_deref()
        .and_then(|u| u.split_once('/').map(|(r, _)| r.to_string()))
        .unwrap_or_else(|| "origin".to_string());
    let remotes = git(repo, &["remote"])?;
    let remotes = String::from_utf8_lossy(&remotes.stdout);
    if !remotes.lines().any(|r| r.trim() == remote) {
        return Err(AppError::Other(format!(
            "no remote named '{remote}' — add one (git remote add origin <url>) before pushing"
        )));
    }
    let set_upstream = upstream.is_none();
    let out = if set_upstream {
        git(repo, &["push", "-u", &remote, &branch])?
    } else {
        git(repo, &["push"])?
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(AppError::Other(format!("git push failed: {err}")));
    }
    let default_branch = default_branch(repo, &remote);
    let pr_url = if default_branch.as_deref() == Some(branch.as_str()) {
        None
    } else {
        remote_url(repo, &remote).and_then(|u| pr_url_for(&u, &branch))
    };
    Ok(PushResult {
        branch,
        remote,
        set_upstream,
        default_branch,
        pr_url,
    })
}

/// The PR/MR link for the current branch as it stands (no push): Some only
/// when the branch has an upstream, is not the remote's default branch, and
/// the host is one we recognize.
pub fn pr_url_current(repo: &Path) -> AppResult<Option<String>> {
    let branch = current_branch(repo)?;
    let Some(upstream) = upstream_of(repo, &branch) else {
        return Ok(None);
    };
    let remote = upstream
        .split_once('/')
        .map(|(r, _)| r.to_string())
        .unwrap_or_else(|| "origin".into());
    if default_branch(repo, &remote).as_deref() == Some(branch.as_str()) {
        return Ok(None);
    }
    Ok(remote_url(repo, &remote).and_then(|u| pr_url_for(&u, &branch)))
}

fn current_branch(repo: &Path) -> AppResult<String> {
    let out = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || b.is_empty() || b == "HEAD" {
        return Err(AppError::Other(
            "not on a branch (detached HEAD) — check out a branch to push".into(),
        ));
    }
    Ok(b)
}

/// `origin/feature` for a tracking branch, None when it has no upstream.
fn upstream_of(repo: &Path, branch: &str) -> Option<String> {
    let out = git(
        repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            &format!("{branch}@{{u}}"),
        ],
    )
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// The remote's default branch from `refs/remotes/<remote>/HEAD`, when the
/// clone recorded it (clones do; a bare `git remote add` doesn't).
fn default_branch(repo: &Path, remote: &str) -> Option<String> {
    let out = git(
        repo,
        &[
            "symbolic-ref",
            "--short",
            &format!("refs/remotes/{remote}/HEAD"),
        ],
    )
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    default_branch_from_symbolic_ref(&s, remote)
}

/// `origin/main` → `main`; tolerant of the bare form.
pub fn default_branch_from_symbolic_ref(short_ref: &str, remote: &str) -> Option<String> {
    let s = short_ref.trim();
    if s.is_empty() {
        return None;
    }
    Some(
        s.strip_prefix(&format!("{remote}/"))
            .unwrap_or(s)
            .to_string(),
    )
}

fn remote_url(repo: &Path, remote: &str) -> Option<String> {
    let out = git(repo, &["remote", "get-url", remote]).ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Turn a remote URL into a "create MR/PR for this branch" web URL for the
/// hosts we recognize (GitHub, GitLab). `None` for anything else. Shared by
/// the Changes tab and the rooms' Share.
pub fn pr_url_for(remote_url: &str, branch: &str) -> Option<String> {
    let (host, path) = parse_remote(remote_url)?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    if host.contains("github") {
        Some(format!("https://{host}/{path}/compare/{branch}?expand=1"))
    } else if host.contains("gitlab") {
        Some(format!(
            "https://{host}/{path}/-/merge_requests/new?merge_request%5Bsource_branch%5D={branch}"
        ))
    } else {
        None
    }
}

/// Split a git remote URL into (host, "owner/repo"). Supports scp-like SSH
/// (`git@host:owner/repo.git`), `ssh://`, and `http(s)://`, stripping any
/// `user@` and the trailing `.git` so it round-trips into a web URL.
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some((host.to_string(), path.to_string()));
    }
    for scheme in ["https://", "http://", "ssh://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            // Drop a leading user@ (ssh URLs) before the host.
            let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
            let (host, path) = rest.split_once('/')?;
            return Some((host.to_string(), path.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod push_tests {
    use super::*;

    #[test]
    fn default_branch_strips_the_remote_prefix() {
        assert_eq!(
            default_branch_from_symbolic_ref("origin/main", "origin").as_deref(),
            Some("main")
        );
        assert_eq!(
            default_branch_from_symbolic_ref("master", "origin").as_deref(),
            Some("master")
        );
        assert_eq!(default_branch_from_symbolic_ref("  ", "origin"), None);
    }

    #[test]
    fn pr_url_for_github_and_gitlab_ssh_and_https() {
        let b = "feat/x";
        assert_eq!(
            pr_url_for("git@github.com:acme/widgets.git", b).as_deref(),
            Some("https://github.com/acme/widgets/compare/feat/x?expand=1")
        );
        assert_eq!(
            pr_url_for("https://github.com/acme/widgets", b).as_deref(),
            Some("https://github.com/acme/widgets/compare/feat/x?expand=1")
        );
        assert_eq!(
            pr_url_for("ssh://git@gitlab.example.com/team/app.git", b).as_deref(),
            Some("https://gitlab.example.com/team/app/-/merge_requests/new?merge_request%5Bsource_branch%5D=feat/x")
        );
        assert_eq!(pr_url_for("git@bitbucket.org:x/y.git", b), None);
    }

    /// Real repos on disk: push into a bare "remote", first with no upstream
    /// (sets it), then with one; the PR link follows the remote's URL shape,
    /// and the default branch yields none.
    #[test]
    fn push_current_sets_upstream_then_pushes_plainly() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("ac-push-{}-{nanos}", std::process::id()));
        let bare = base.join("remote.git");
        let work = base.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let run = |dir: &Path, args: &[&str]| {
            let out = proc::command("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&base, &["init", "--bare", "-q", "-b", "main", "remote.git"]);
        run(&work, &["init", "-q", "-b", "main"]);
        run(&work, &["config", "user.email", "t@t"]);
        run(&work, &["config", "user.name", "t"]);
        std::fs::write(work.join("a.txt"), "a").unwrap();
        run(&work, &["add", "a.txt"]);
        run(&work, &["commit", "-q", "-m", "init"]);
        // A GitHub-shaped URL for the link; the fetch/push URL is the local bare repo.
        run(
            &work,
            &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
        );
        run(
            &work,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                bare.to_str().unwrap(),
            ],
        );
        run(
            &work,
            &["remote", "set-url", "origin", bare.to_str().unwrap()],
        );
        run(
            &work,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                bare.to_str().unwrap(),
            ],
        );

        // main: first push creates the upstream; no PR link on the default
        // branch once origin/HEAD is known (set it as a clone would).
        let r = push_current(&work).unwrap();
        assert_eq!(r.branch, "main");
        assert_eq!(r.remote, "origin");
        assert!(r.set_upstream);
        run(&work, &["remote", "set-head", "origin", "main"]);

        // A feature branch: -u the first time, plain push after, PR link
        // shaped by the remote URL — swap the URL to the GitHub form to check.
        run(&work, &["checkout", "-q", "-b", "feat/x"]);
        std::fs::write(work.join("b.txt"), "b").unwrap();
        run(&work, &["add", "b.txt"]);
        run(&work, &["commit", "-q", "-m", "feat"]);
        let r = push_current(&work).unwrap();
        assert!(r.set_upstream);
        assert_eq!(r.default_branch.as_deref(), Some("main"));
        assert_eq!(r.pr_url, None, "a local path remote is no known host");
        run(
            &work,
            &[
                "remote",
                "set-url",
                "origin",
                "git@github.com:acme/widgets.git",
            ],
        );
        run(
            &work,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                bare.to_str().unwrap(),
            ],
        );
        std::fs::write(work.join("c.txt"), "c").unwrap();
        run(&work, &["add", "c.txt"]);
        run(&work, &["commit", "-q", "-m", "more"]);
        let r = push_current(&work).unwrap();
        assert!(!r.set_upstream, "second push is plain");
        assert_eq!(
            r.pr_url.as_deref(),
            Some("https://github.com/acme/widgets/compare/feat/x?expand=1")
        );
        assert_eq!(pr_url_current(&work).unwrap(), r.pr_url);
        // The default branch never offers a PR.
        run(&work, &["checkout", "-q", "main"]);
        assert_eq!(pr_url_current(&work).unwrap(), None);
        let _ = std::fs::remove_dir_all(&base);
    }
}
