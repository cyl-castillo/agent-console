use crate::services::proc;
use std::path::Path;

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
    let _ = proc::command("git")
        .env("GIT_INDEX_FILE", &tmp_idx_str)
        .args(["read-tree", "HEAD"])
        .current_dir(repo)
        .output();

    // Stage everything from working tree (honors .gitignore).
    let add = proc::command("git")
        .env("GIT_INDEX_FILE", &tmp_idx_str)
        .args(["add", "-A"])
        .current_dir(repo)
        .output()?;
    if !add.status.success() {
        cleanup(&tmp_idx);
        return Err(AppError::Other(format!(
            "snapshot add: {}",
            String::from_utf8_lossy(&add.stderr)
        )));
    }

    // Capture tree.
    let tree_out = proc::command("git")
        .env("GIT_INDEX_FILE", &tmp_idx_str)
        .args(["write-tree"])
        .current_dir(repo)
        .output()?;
    cleanup(&tmp_idx);
    if !tree_out.status.success() {
        return Err(AppError::Other(format!(
            "write-tree: {}",
            String::from_utf8_lossy(&tree_out.stderr)
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

    let commit_out = proc::command("git")
        .args(&commit_args)
        .current_dir(repo)
        .output()?;
    if !commit_out.status.success() {
        return Err(AppError::Other(format!(
            "commit-tree: {}",
            String::from_utf8_lossy(&commit_out.stderr)
        )));
    }
    let commit_sha = String::from_utf8_lossy(&commit_out.stdout)
        .trim()
        .to_string();

    // Pin via ref so GC won't collect it.
    let _ = proc::command("git")
        .args(["update-ref", &snapshot_ref(id), &commit_sha])
        .current_dir(repo)
        .output()?;

    Ok(Some(Snapshot {
        id: id.to_string(),
        commit_sha,
    }))
}

/// Force working tree + index to match this snapshot's tree. Doesn't move HEAD.
pub fn restore(repo: &Path, commit_sha: &str) -> AppResult<()> {
    // Resolve the tree from the commit.
    let tree = proc::command("git")
        .args(["rev-parse", &format!("{commit_sha}^{{tree}}")])
        .current_dir(repo)
        .output()?;
    if !tree.status.success() {
        return Err(AppError::Other(format!(
            "snapshot not found: {}",
            String::from_utf8_lossy(&tree.stderr)
        )));
    }
    let tree_sha = String::from_utf8_lossy(&tree.stdout).trim().to_string();

    let out = proc::command("git")
        .args(["read-tree", "--reset", "-u", &tree_sha])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "read-tree restore: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

/// Files that changed between two snapshot commits, as (status, path) pairs
/// ("M"/"A"/"D"/"R100 old\tnew"...). Capped so a huge turn (vendored deps,
/// generated code) can't balloon the caller's record; the cap is reported by
/// the caller, not silently here.
pub const DIFF_NAMES_MAX: usize = 500;

pub fn diff_names(repo: &Path, from_sha: &str, to_sha: &str) -> AppResult<Vec<(String, String)>> {
    let out = proc::command("git")
        .args(["diff", "--name-status", from_sha, to_sha])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "diff --name-status: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(2, '\t');
            let status = parts.next()?.trim().to_string();
            let path = parts.next()?.trim().to_string();
            (!status.is_empty() && !path.is_empty()).then_some((status, path))
        })
        .take(DIFF_NAMES_MAX)
        .collect())
}

pub fn delete(repo: &Path, id: &str) -> AppResult<()> {
    let _ = proc::command("git")
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
    let out = proc::command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo)
        .output()?;
    Ok(out.status.success())
}

fn head_sha(repo: &Path) -> Option<String> {
    let out = proc::command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
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

    fn init_repo(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo = std::env::temp_dir().join(format!("ac-snap-{tag}-{nanos}"));
        fs::create_dir_all(&repo).unwrap();
        git(&["init", "-q"], &repo);
        git(&["config", "user.email", "t@t"], &repo);
        git(&["config", "user.name", "T"], &repo);
        git(&["config", "commit.gpgsign", "false"], &repo);
        repo
    }

    #[test]
    fn non_repo_yields_no_snapshot() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let plain = std::env::temp_dir().join(format!("ac-snap-norepo-{nanos}"));
        fs::create_dir_all(&plain).unwrap();
        assert!(create(&plain, "x").unwrap().is_none());
        let _ = fs::remove_dir_all(&plain);
    }

    #[test]
    fn snapshot_lifecycle_create_restore_delete() {
        let repo = init_repo("life");
        fs::write(repo.join("tracked.txt"), "v1").unwrap();
        git(&["add", "-A"], &repo);
        git(&["commit", "-qm", "seed"], &repo);

        // State to capture: a tracked edit AND an untracked file. The snapshot
        // must include both (that's its whole point vs. plain stash-like flows).
        fs::write(repo.join("tracked.txt"), "v2").unwrap();
        fs::write(repo.join("untracked.txt"), "new").unwrap();
        let snap = create(&repo, "turn-1").unwrap().expect("repo → snapshot");
        assert!(!snap.commit_sha.is_empty());

        // Pinned via ref (GC-safe), temp index cleaned up, HEAD untouched.
        let r = git(&["rev-parse", "refs/agent-console/turn-1"], &repo);
        assert!(r.status.success(), "snapshot ref must exist");
        assert_eq!(
            String::from_utf8_lossy(&r.stdout).trim(),
            snap.commit_sha,
            "ref points at the snapshot commit"
        );
        assert!(
            !repo.join(".git/agent-console-idx-turn-1").exists(),
            "temp index file is cleaned up"
        );
        let head = git(&["log", "-1", "--format=%s"], &repo);
        assert_eq!(
            String::from_utf8_lossy(&head.stdout).trim(),
            "seed",
            "HEAD never moves"
        );

        // Wreck the tree after the snapshot, then restore to it.
        fs::write(repo.join("tracked.txt"), "v3-bad").unwrap();
        fs::remove_file(repo.join("untracked.txt")).unwrap();
        restore(&repo, &snap.commit_sha).unwrap();
        assert_eq!(fs::read_to_string(repo.join("tracked.txt")).unwrap(), "v2");
        assert_eq!(
            fs::read_to_string(repo.join("untracked.txt")).unwrap(),
            "new",
            "untracked-at-snapshot files come back on restore"
        );

        // Restore of a bogus sha is a clear error, not silent corruption.
        assert!(restore(&repo, "0000000000000000000000000000000000000000").is_err());

        // Delete drops the pin; deleting again is idempotent.
        delete(&repo, "turn-1").unwrap();
        let r = git(&["rev-parse", "refs/agent-console/turn-1"], &repo);
        assert!(!r.status.success(), "ref removed");
        delete(&repo, "turn-1").unwrap();

        let _ = fs::remove_dir_all(&repo);
    }

    // Mirrors what the snapshot_restore command does: back up the current tree
    // before the destructive restore, so the restore itself can be undone. The
    // at-risk work is post-snapshot edits to TRACKED files — `read-tree --reset -u`
    // overwrites those (untracked new files are left in place).
    #[test]
    fn pre_restore_backup_makes_restore_undoable() {
        let repo = init_repo("undo");
        fs::write(repo.join("f.txt"), "committed").unwrap();
        git(&["add", "-A"], &repo);
        git(&["commit", "-qm", "seed"], &repo);

        // Snapshot A — the "good" state we'll later wind back to.
        fs::write(repo.join("f.txt"), "good").unwrap();
        let a = create(&repo, "A").unwrap().unwrap();

        // A tracked edit past A — exactly the post-snapshot work a restore destroys.
        fs::write(repo.join("f.txt"), "later-work").unwrap();

        // Back up the CURRENT tree, then restore A (destructive: "later-work" gone).
        let backup = create(&repo, "pre-restore").unwrap().unwrap();
        restore(&repo, &a.commit_sha).unwrap();
        assert_eq!(
            fs::read_to_string(repo.join("f.txt")).unwrap(),
            "good",
            "restore wound the tracked edit back to A"
        );

        // Undo = restore the backup → the post-A edit comes back, nothing lost.
        restore(&repo, &backup.commit_sha).unwrap();
        assert_eq!(
            fs::read_to_string(repo.join("f.txt")).unwrap(),
            "later-work",
            "undo brings back the work the restore had wound past"
        );

        let _ = fs::remove_dir_all(&repo);
    }
}
