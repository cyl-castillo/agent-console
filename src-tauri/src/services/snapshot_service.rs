use crate::services::proc;
use std::collections::HashSet;
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
        // A missing object here is usually retention, not corruption: the pin
        // expired (see `sweep`) and the user's own `git gc` reclaimed the tree.
        // Say so, because "snapshot not found" alone reads like a bug.
        return Err(AppError::Other(format!(
            "snapshot not found — snapshots stay restorable for {SNAPSHOT_RETENTION_DAYS} days; \
             past that the ledger keeps the record but the tree is gone: {}",
            String::from_utf8_lossy(&tree.stderr).trim()
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

/// How long an auto-snapshot ref stays pinned. Two snapshots per turn (pre at
/// the prompt, post at Stop/StopFailure) pin whole trees forever otherwise, and
/// in a repo with real churn that only ever grows. Thirty days matches what
/// Claude Code itself keeps for its file checkpoints (`cleanupPeriodDays`), so
/// the undo we offer outlives the CLI's own.
pub const SNAPSHOT_RETENTION_DAYS: u64 = 30;

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// What a retention pass did. Reported, never guessed at: the caller logs it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    pub deleted: usize,
    pub kept: usize,
}

/// Drop the pins on auto-snapshots older than `max_age_days`, so the objects
/// become unreachable and the user's own `git gc` can reclaim them. We never
/// run `gc` ourselves — repacking someone's repo behind their back is not our
/// call; we only stop holding trees alive.
///
/// What survives, by construction:
/// - **Refs we didn't auto-create**: only a uuid-named ref is swept, because
///   that is exactly the shape `create` is called with per turn. That leaves
///   `refs/agent-console/testigo-head` (the ledger anchor) and the
///   `pre-restore-<nanos>` backups (the undo of a destructive restore, one per
///   explicit user action) untouched, without either needing a special case.
/// - **Anything in `keep_shas`**: the caller passes the pre/post pair of the
///   most recent turn per terminal — the undo that is still live.
/// - **Anything younger than the cutoff.**
///
/// What is NOT at risk: the proof ledger. Its evidence is the recorded shas and
/// the file list, materialized at turn_end and hash-chained; export reads ledger
/// lines, never git objects. Losing the pin costs the ability to restore or
/// re-diff an old tree, not the record that it existed.
///
/// `now_unix` is a parameter so the age boundary is testable without waiting a
/// month. In a linked worktree this still covers everything: `refs/agent-console/*`
/// is not a per-worktree ref, so worktree sessions write into the common ref
/// store that the project root sweeps.
pub fn sweep(
    repo: &Path,
    keep_shas: &HashSet<String>,
    max_age_days: u64,
    now_unix: i64,
) -> AppResult<SweepReport> {
    if !is_git_repo(repo)? {
        return Ok(SweepReport::default());
    }
    let cutoff = now_unix - (max_age_days as i64) * SECONDS_PER_DAY;

    let out = proc::command("git")
        .args([
            "for-each-ref",
            "--format=%(refname)\t%(objectname)\t%(creatordate:unix)",
            "refs/agent-console/",
        ])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "for-each-ref: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }

    let mut report = SweepReport::default();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.split('\t');
        let (Some(refname), Some(oid), Some(created)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Some(id) = refname.strip_prefix("refs/agent-console/") else {
            continue;
        };
        // A ref whose creation date we can't read is a ref we don't delete.
        let created: i64 = match created.trim().parse() {
            Ok(c) => c,
            Err(_) => {
                report.kept += 1;
                continue;
            }
        };
        if !is_auto_snapshot_id(id) || created > cutoff || keep_shas.contains(oid) {
            report.kept += 1;
            continue;
        }
        // Pass the old value: if something re-pointed this ref since the listing,
        // the delete fails instead of dropping a pin we never looked at.
        let del = proc::command("git")
            .args(["update-ref", "-d", refname, oid])
            .current_dir(repo)
            .output()?;
        if del.status.success() {
            report.deleted += 1;
        } else {
            report.kept += 1;
        }
    }
    Ok(report)
}

/// True for the uuid shape `create` is fed once per snapshot (8-4-4-4-12 hex).
/// Deliberately narrow: a ref this doesn't recognize is a ref we leave alone.
fn is_auto_snapshot_id(id: &str) -> bool {
    let groups = [8, 4, 4, 4, 12];
    let mut parts = id.split('-');
    for len in groups {
        match parts.next() {
            Some(p) if p.len() == len && p.chars().all(|c| c.is_ascii_hexdigit()) => {}
            _ => return false,
        }
    }
    parts.next().is_none()
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

    #[test]
    fn only_uuid_shaped_ids_are_recognized_as_auto_snapshots() {
        assert!(is_auto_snapshot_id("450aeb31-8079-498b-afab-1f0fab67b3e7"));
        // Everything we or a user might park in the namespace by hand.
        for id in [
            "testigo-head",
            "pre-restore-1756800000000000000",
            "450aeb31-8079-498b-afab-1f0fab67b3e", // short group
            "450aeb31-8079-498b-afab-1f0fab67b3e77", // long group
            "450aeb31-8079-498b-afab",             // missing group
            "450aeb31-8079-498b-afab-1f0fab67b3e7-x", // extra group
            "450aeb31_8079_498b_afab_1f0fab67b3e7", // wrong separator
            "450aeb31-8079-498b-afab-1f0fab67b3zz", // non-hex
            "",
        ] {
            assert!(!is_auto_snapshot_id(id), "{id:?} must not be swept");
        }
    }

    #[test]
    fn sweep_expires_old_pins_but_never_the_ones_still_load_bearing() {
        let repo = init_repo("sweep");
        fs::write(repo.join("f.txt"), "seed").unwrap();
        git(&["add", "-A"], &repo);
        git(&["commit", "-qm", "seed"], &repo);

        let old = create(&repo, "11111111-1111-4111-8111-111111111111")
            .unwrap()
            .unwrap();
        fs::write(repo.join("f.txt"), "later").unwrap();
        let live_undo = create(&repo, "22222222-2222-4222-8222-222222222222")
            .unwrap()
            .unwrap();
        // Refs we never auto-created: the ledger anchor and a restore backup.
        git(
            &["update-ref", "refs/agent-console/testigo-head", "HEAD"],
            &repo,
        );
        git(
            &["update-ref", "refs/agent-console/pre-restore-42", "HEAD"],
            &repo,
        );

        let now = now_unix();
        let keep: HashSet<String> = [live_undo.commit_sha.clone()].into_iter().collect();

        // Nothing is old enough yet: a sweep at creation time is a no-op.
        let fresh = sweep(&repo, &keep, 30, now).unwrap();
        assert_eq!(fresh.deleted, 0, "young pins are never touched");
        let pinned = git(
            &[
                "rev-parse",
                "refs/agent-console/11111111-1111-4111-8111-111111111111",
            ],
            &repo,
        );
        assert_eq!(
            String::from_utf8_lossy(&pinned.stdout).trim(),
            old.commit_sha,
            "still pinned at its snapshot commit"
        );

        // Same refs, 40 days later: only the expired auto-snapshot goes.
        let later = now + 40 * SECONDS_PER_DAY;
        let report = sweep(&repo, &keep, 30, later).unwrap();
        assert_eq!(report.deleted, 1, "exactly the expired, unreferenced pin");
        assert_eq!(report.kept, 3, "live undo + anchor + restore backup");

        assert!(
            !git(
                &[
                    "rev-parse",
                    "refs/agent-console/11111111-1111-4111-8111-111111111111"
                ],
                &repo
            )
            .status
            .success(),
            "expired pin dropped"
        );
        for still in [
            "refs/agent-console/22222222-2222-4222-8222-222222222222",
            "refs/agent-console/testigo-head",
            "refs/agent-console/pre-restore-42",
        ] {
            assert!(
                git(&["rev-parse", still], &repo).status.success(),
                "{still} must survive retention"
            );
        }

        // The kept snapshot is still a working undo, not just a live ref.
        fs::write(repo.join("f.txt"), "wrecked").unwrap();
        restore(&repo, &live_undo.commit_sha).unwrap();
        assert_eq!(fs::read_to_string(repo.join("f.txt")).unwrap(), "later");

        // And the swept one now fails with a reason that names retention
        // instead of reading like corruption. (The object may still be in the
        // odb until the user's own gc runs, so assert on the message we'd give
        // for an unresolvable sha.)
        let err = restore(&repo, "0000000000000000000000000000000000000000")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("30 days"),
            "restore error explains retention: {err}"
        );

        // Idempotent: a second pass has nothing left to expire.
        let again = sweep(&repo, &keep, 30, later).unwrap();
        assert_eq!(again.deleted, 0);

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn sweep_of_a_plain_directory_is_a_no_op() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let plain = std::env::temp_dir().join(format!("ac-snap-sweep-norepo-{nanos}"));
        fs::create_dir_all(&plain).unwrap();
        let r = sweep(&plain, &HashSet::new(), 30, now_unix()).unwrap();
        assert_eq!(r, SweepReport::default());
        let _ = fs::remove_dir_all(&plain);
    }

    fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
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
