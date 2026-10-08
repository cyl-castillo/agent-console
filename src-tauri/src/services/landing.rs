//! Landing a job's worktree onto its base branch.
//!
//! Port of ai-connector's `integrations/git_landing.py` and the Git half of
//! `execution/landing.py` (Marcos Macías, with permission). The shape is his:
//! commit the worktree, bring the base up to date from its upstream (fast
//! forward only), merge the base INTO the job branch (conflicts are resolved
//! there, never in the user's checkout), then land by fast-forwarding the base
//! with a compare-and-swap on the commit the caller last saw. The user's
//! checkout is touched only when it sits on the base branch, and then only
//! with `merge --ff-only`, which refuses to overwrite local edits.
//!
//! Everything here is plain Git over `std::process`; the room driver owns the
//! turns (conflict resolution, re-review) and the persisted [`LandingState`].

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::services::proc;

/// Why landing cannot proceed. The message is what the job's card shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandingError(pub String);

impl fmt::Display for LandingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

type LResult<T> = Result<T, LandingError>;

/// The base and job commits a prepared merge joined, so an unchanged pair can
/// skip preparation on re-entry (ai-connector's `worktree_state.synced`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Synced {
    pub base: String,
    pub head: String,
}

/// Durable landing state of one job, persisted with the room.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LandingState {
    pub base_branch: String,
    pub branch: String,
    /// The job worktree.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced: Option<Synced>,
    /// Files still conflicted after the last merge attempt.
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// Conflict-resolution turns spent (bounded by `max_corrections`).
    #[serde(default)]
    pub resolutions: u32,
    /// Times the base moved under a prepared merge (bounded to three).
    #[serde(default)]
    pub rounds: u32,
    /// The job revision the current `synced` was prepared for; a newer
    /// revision (a correction turn) re-prepares.
    #[serde(default)]
    pub phase_revision: String,
    /// `true` once the branch landed and the worktree/branch were cleaned up.
    #[serde(default)]
    pub landed: bool,
}

impl LandingState {
    pub fn new(base_branch: &str, branch: &str, path: &Path) -> Self {
        Self {
            base_branch: base_branch.into(),
            branch: branch.into(),
            path: path.display().to_string(),
            synced: None,
            conflicts: Vec::new(),
            resolutions: 0,
            rounds: 0,
            phase_revision: String::new(),
            landed: false,
        }
    }
}

/// Outcome of [`prepare`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prepared {
    /// Base merged into the job branch; `new_commit` says whether the merge
    /// (or a completed resolution) produced a commit the review has not seen.
    Ready { new_commit: bool },
    /// The merge stopped on these files; a resolution turn or the human must
    /// clear the markers, then [`prepare`] again completes the merge.
    Conflicts(Vec<String>),
}

/// How many times a moved base is re-merged before the job pauses.
pub const MAX_ROUNDS: u32 = 3;

// ---------------------------------------------------------------------------
// Git plumbing
// ---------------------------------------------------------------------------

fn git_raw(cwd: &Path, args: &[&str], identity: bool) -> LResult<std::process::Output> {
    let mut cmd = proc::command("git");
    if identity {
        for (key, fallback) in [
            ("user.name", "Agent Console"),
            ("user.email", "agent-console@localhost"),
        ] {
            if !configured(cwd, key) {
                cmd.arg("-c").arg(format!("{key}={fallback}"));
            }
        }
    }
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| LandingError(format!("Git failed while landing: {e}")))
}

fn configured(cwd: &Path, key: &str) -> bool {
    proc::command("git")
        .args(["config", "--get", key])
        .current_dir(cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn failure(out: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if err.is_empty() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        err
    }
}

fn git(cwd: &Path, args: &[&str]) -> LResult<String> {
    let out = git_raw(cwd, args, false)?;
    if !out.status.success() {
        return Err(LandingError(format!(
            "Git failed while landing: git {}: {}",
            args.join(" "),
            failure(&out)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Commit of `refs/heads/<branch>`, or an error naming a missing branch.
pub fn base_tip(repo: &Path, branch: &str) -> LResult<String> {
    let out = git_raw(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}^{{commit}}"),
        ],
        false,
    )?;
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        Some(1) => Err(LandingError(format!(
            "The base branch {branch} no longer exists"
        ))),
        _ => Err(LandingError(format!(
            "Git failed while landing: {}",
            failure(&out)
        ))),
    }
}

/// The job branch tip, or `None` once the branch was deleted.
pub fn job_tip(repo: &Path, branch: &str) -> LResult<Option<String>> {
    let out = git_raw(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}^{{commit}}"),
        ],
        false,
    )?;
    match out.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        )),
        Some(1) => Ok(None),
        _ => Err(LandingError(format!(
            "Git failed while landing: {}",
            failure(&out)
        ))),
    }
}

pub fn is_ancestor(repo: &Path, ancestor: &str, descendant: &str) -> LResult<bool> {
    let out = git_raw(
        repo,
        &["merge-base", "--is-ancestor", ancestor, descendant],
        false,
    )?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(LandingError(format!(
            "Git failed while landing: {}",
            failure(&out)
        ))),
    }
}

/// Where `branch` is checked out, if anywhere — refusing the job's own worktree.
pub fn branch_checkout(
    repo: &Path,
    branch: &str,
    job_path: Option<&Path>,
) -> LResult<Option<PathBuf>> {
    let listing = git(repo, &["worktree", "list", "--porcelain"])?;
    let want = format!("refs/heads/{branch}");
    let mut path: Option<PathBuf> = None;
    for line in listing.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch ") {
            if b == want {
                let p = path.clone().unwrap_or_default();
                if let Some(job) = job_path {
                    if same_path(&p, job) {
                        return Err(LandingError(
                            "The base branch is checked out in the job worktree".into(),
                        ));
                    }
                }
                return Ok(Some(p));
            }
        }
    }
    Ok(None)
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn upstream_remote(repo: &Path, branch: &str) -> LResult<Option<String>> {
    let out = git_raw(
        repo,
        &["config", "--get", &format!("branch.{branch}.remote")],
        false,
    )?;
    match out.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        )),
        Some(1) => Ok(None),
        _ => Err(LandingError(format!(
            "Git failed while landing: {}",
            failure(&out)
        ))),
    }
}

/// Fetch and fast-forward the base from its upstream (when it has one).
/// Returns the base commit afterwards. A diverged base is the human's call.
pub fn update_base(repo: &Path, state: &LandingState) -> LResult<String> {
    let branch = &state.base_branch;
    let before = base_tip(repo, branch)?;
    let Some(remote) = upstream_remote(repo, branch)? else {
        return Ok(before);
    };
    if let Err(e) = git(repo, &["fetch", "--", &remote]) {
        return Err(LandingError(format!(
            "Could not update the base branch from its remote: {e}"
        )));
    }
    let upstream = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            &format!("{branch}@{{upstream}}^{{commit}}"),
        ],
    )?;
    if is_ancestor(repo, &upstream, &before)? {
        return Ok(before);
    }
    if !is_ancestor(repo, &before, &upstream)? {
        return Err(LandingError(format!(
            "The base branch {branch} has diverged from its remote; reconcile it manually, then continue"
        )));
    }
    match branch_checkout(repo, branch, Some(Path::new(&state.path)))? {
        Some(checkout) => {
            git(
                &checkout,
                &["merge", "--ff-only", "--no-autostash", &upstream],
            )
            .map_err(|e| LandingError(format!("The base branch cannot be updated: {e}")))?;
        }
        None => {
            git(
                repo,
                &[
                    "update-ref",
                    &format!("refs/heads/{branch}"),
                    &upstream,
                    &before,
                ],
            )?;
        }
    }
    base_tip(repo, branch)
}

pub fn merge_in_progress(path: &Path) -> bool {
    git_raw(
        path,
        &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"],
        false,
    )
    .map(|o| o.status.success())
    .unwrap_or(false)
}

pub fn unmerged_files(path: &Path) -> LResult<Vec<String>> {
    Ok(git(path, &["diff", "--name-only", "--diff-filter=U"])?
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Among `files`, those still carrying conflict markers.
pub fn conflict_marker_files(path: &Path, files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|f| {
            std::fs::read_to_string(path.join(f))
                .map(|t| {
                    t.lines().any(|l| {
                        l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "======="
                    })
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// Commit everything in the worktree (one checkpoint). `true` when a commit was made.
pub fn commit_all(path: &Path, message: &str) -> LResult<bool> {
    git(path, &["add", "-A"])?;
    let staged = git_raw(path, &["diff", "--cached", "--quiet"], false)?;
    if staged.status.success() {
        return Ok(false);
    }
    let out = git_raw(path, &["commit", "-q", "-m", message], true)?;
    if !out.status.success() {
        return Err(LandingError(format!(
            "Git failed while landing: {}",
            failure(&out)
        )));
    }
    Ok(true)
}

/// Finish an in-progress merge once no file keeps markers: stage the formerly
/// conflicted files and commit. `false` leaves the merge open for another try.
pub fn complete_merge(path: &Path, files: &[String]) -> LResult<bool> {
    if !conflict_marker_files(path, files).is_empty() {
        return Ok(false);
    }
    for f in files {
        // A file deleted on one side is resolved by removing it.
        if path.join(f).exists() {
            git(path, &["add", "--", f])?;
        } else {
            git(path, &["rm", "-q", "--cached", "--ignore-unmatch", "--", f])?;
        }
    }
    if !unmerged_files(path)?.is_empty() {
        return Ok(false);
    }
    let out = git_raw(path, &["commit", "-q", "--no-edit"], true)?;
    if !out.status.success() {
        return Err(LandingError(format!(
            "Git failed while landing: {}",
            failure(&out)
        )));
    }
    Ok(true)
}

/// Merge `base_commit` into the worktree's branch. Returns the conflicting
/// paths (empty for a clean merge or when already up to date).
pub fn merge_base(path: &Path, base_commit: &str) -> LResult<Vec<String>> {
    if merge_in_progress(path) {
        return unmerged_files(path);
    }
    let out = git_raw(
        path,
        &[
            "merge",
            "--no-autostash",
            "--no-edit",
            "--no-squash",
            base_commit,
        ],
        true,
    )?;
    if out.status.success() {
        return Ok(Vec::new());
    }
    let files = unmerged_files(path)?;
    if !files.is_empty() && merge_in_progress(path) {
        return Ok(files);
    }
    Err(LandingError(format!(
        "Git failed while landing: {}",
        failure(&out)
    )))
}

// ---------------------------------------------------------------------------
// The phases
// ---------------------------------------------------------------------------

/// Commit the worktree, update the base and merge it into the job branch.
/// Idempotent: an unchanged (base, head) pair is `Ready` at once; an open merge
/// is completed when its conflicts were cleared, or reported again.
pub fn prepare(
    repo: &Path,
    state: &mut LandingState,
    checkpoint_message: &str,
) -> LResult<Prepared> {
    let path = PathBuf::from(&state.path);
    if !path.is_dir() {
        return Err(LandingError(format!(
            "The job worktree at {} is gone; the branch {} keeps the work",
            state.path, state.branch
        )));
    }
    let mut new_commit = false;
    if merge_in_progress(&path) {
        let mut files = state.conflicts.clone();
        for f in unmerged_files(&path)? {
            if !files.contains(&f) {
                files.push(f);
            }
        }
        if !complete_merge(&path, &files)? {
            let remaining: Vec<String> = {
                let mut r = conflict_marker_files(&path, &files);
                for f in unmerged_files(&path)? {
                    if !r.contains(&f) {
                        r.push(f);
                    }
                }
                r
            };
            state.conflicts = remaining.clone();
            return Ok(Prepared::Conflicts(remaining));
        }
        state.conflicts.clear();
        new_commit = true;
    } else if !state.conflicts.is_empty() {
        // The human may have resolved by hand and committed; nothing is open.
        state.conflicts.clear();
    }
    if commit_all(&path, checkpoint_message)? {
        new_commit = true;
    }
    let head = job_tip(repo, &state.branch)?
        .ok_or_else(|| LandingError(format!("The job branch {} no longer exists", state.branch)))?;
    let base = base_tip(repo, &state.base_branch)?;
    if state
        .synced
        .as_ref()
        .is_some_and(|s| s.base == base && s.head == head)
    {
        return Ok(Prepared::Ready { new_commit });
    }
    let base = update_base(repo, state)?;
    let files = merge_base(&path, &base)?;
    if !files.is_empty() {
        state.conflicts = files.clone();
        return Ok(Prepared::Conflicts(files));
    }
    let merged_head = job_tip(repo, &state.branch)?.unwrap_or(head.clone());
    if merged_head != head {
        new_commit = true;
    }
    state.synced = Some(Synced {
        base,
        head: merged_head,
    });
    Ok(Prepared::Ready { new_commit })
}

/// Land: fast-forward the base to the prepared job head, compare-and-swap on
/// the base commit the merge saw. `Ok(false)` = the base moved; prepare again.
pub fn land(repo: &Path, state: &LandingState) -> LResult<bool> {
    let Some(synced) = &state.synced else {
        return Err(LandingError("Nothing prepared to land".into()));
    };
    let branch = &state.base_branch;
    let before = base_tip(repo, branch)?;
    let head = job_tip(repo, &state.branch)?.unwrap_or(synced.head.clone());
    if is_ancestor(repo, &head, &before)? {
        return Ok(true);
    }
    if before != synced.base {
        return Ok(false);
    }
    if !is_ancestor(repo, &before, &head)? {
        return Err(LandingError(
            "The base branch cannot be updated: the job branch does not contain its tip".into(),
        ));
    }
    match branch_checkout(repo, branch, Some(Path::new(&state.path)))? {
        Some(checkout) => {
            // The user's checkout sits on the base: fast-forward it in place.
            // `--ff-only` refuses when local edits would be overwritten, which
            // is exactly the pause ai-connector reports.
            let out = git_raw(
                &checkout,
                &["merge", "--ff-only", "--no-autostash", &head],
                false,
            )?;
            if !out.status.success() {
                let current = base_tip(repo, branch)?;
                if is_ancestor(repo, &head, &current)? {
                    return Ok(true);
                }
                return Err(LandingError(format!(
                    "The base branch cannot be updated: {}",
                    failure(&out)
                )));
            }
        }
        None => {
            git(
                repo,
                &[
                    "update-ref",
                    &format!("refs/heads/{branch}"),
                    &head,
                    &before,
                ],
            )?;
        }
    }
    Ok(true)
}

/// Remove the worktree and delete the job branch, only once it is contained
/// in the base. Idempotent for a half-done cleanup.
pub fn cleanup_landed(repo: &Path, state: &LandingState) -> LResult<()> {
    let base = base_tip(repo, &state.base_branch)?;
    if let Some(head) = job_tip(repo, &state.branch)? {
        if !is_ancestor(repo, &head, &base)? {
            return Err(LandingError(format!(
                "The job branch {} has not landed on the base branch",
                state.branch
            )));
        }
    }
    let path = Path::new(&state.path);
    if path.exists() {
        let _ = git_raw(repo, &["worktree", "remove", "--force", &state.path], false);
    }
    let _ = git_raw(repo, &["worktree", "prune"], false);
    if job_tip(repo, &state.branch)?.is_some() {
        git(repo, &["branch", "-D", &state.branch])?;
    }
    Ok(())
}

/// Per-project landing lock: one landing (or base update) at a time per repo,
/// the equivalent of ai-connector's `merge.lock`.
pub fn lock(project: &str) -> Arc<Mutex<()>> {
    static LOCKS: Mutex<Option<HashMap<String, Arc<Mutex<()>>>>> = Mutex::new(None);
    let mut map = LOCKS.lock();
    map.get_or_insert_with(HashMap::new)
        .entry(project.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sh(cwd: &Path, args: &[&str]) -> String {
        let out = proc::command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repo on `main` with one seed commit, plus a job worktree on `job/x`.
    fn scenario(tag: &str) -> (PathBuf, PathBuf, LandingState) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo = std::env::temp_dir().join(format!("ac-landing-{tag}-{nanos}"));
        fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "T"]);
        fs::write(repo.join("a.txt"), "a\n").unwrap();
        fs::write(repo.join("b.txt"), "b\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "seed"]);
        let wt = repo.join(".wt-job");
        sh(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "job/x",
                wt.to_str().unwrap(),
                "main",
            ],
        );
        let state = LandingState::new("main", "job/x", &wt);
        (repo, wt, state)
    }

    #[test]
    fn clean_landing_fast_forwards_base_and_cleans_up() {
        let (repo, wt, mut state) = scenario("clean");
        fs::write(wt.join("job.txt"), "work\n").unwrap();
        // Uncommitted work is checkpointed by prepare.
        let prepared = prepare(&repo, &mut state, "job checkpoint").unwrap();
        assert_eq!(prepared, Prepared::Ready { new_commit: true });
        let synced = state.synced.clone().unwrap();
        assert_eq!(synced.base, base_tip(&repo, "main").unwrap());
        assert_eq!(Some(synced.head.clone()), job_tip(&repo, "job/x").unwrap());
        // Re-entering with nothing new is Ready without a new commit.
        assert_eq!(
            prepare(&repo, &mut state, "noop").unwrap(),
            Prepared::Ready { new_commit: false }
        );
        // The main checkout sits on `main`: landing fast-forwards it in place.
        assert!(land(&repo, &state).unwrap());
        assert_eq!(base_tip(&repo, "main").unwrap(), synced.head);
        assert!(repo.join("job.txt").exists(), "checkout updated");
        cleanup_landed(&repo, &state).unwrap();
        assert!(!wt.exists());
        assert_eq!(job_tip(&repo, "job/x").unwrap(), None);
        // Idempotent.
        cleanup_landed(&repo, &state).unwrap();
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_moved_base_is_merged_in_and_a_second_move_is_reported_by_cas() {
        let (repo, wt, mut state) = scenario("moved");
        fs::write(wt.join("job.txt"), "work\n").unwrap();
        sh(&wt, &["add", "-A"]);
        sh(&wt, &["commit", "-qm", "job"]);
        // Someone commits on main meanwhile (no overlap with the job).
        fs::write(repo.join("c.txt"), "c\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "main moves"]);
        let prepared = prepare(&repo, &mut state, "cp").unwrap();
        assert_eq!(
            prepared,
            Prepared::Ready { new_commit: true },
            "merge commit created"
        );
        assert!(wt.join("c.txt").exists(), "base merged into the job branch");
        // The base moves AGAIN after preparation: land refuses (CAS) instead of
        // overwriting, and a fresh prepare picks the new base up.
        fs::write(repo.join("d.txt"), "d\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "main moves again"]);
        assert!(!land(&repo, &state).unwrap());
        assert_eq!(
            prepare(&repo, &mut state, "cp").unwrap(),
            Prepared::Ready { new_commit: true }
        );
        assert!(land(&repo, &state).unwrap());
        assert!(repo.join("job.txt").exists() && repo.join("d.txt").exists());
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn conflicts_are_reported_then_completed_once_markers_are_gone() {
        let (repo, wt, mut state) = scenario("conflict");
        fs::write(wt.join("a.txt"), "job version\n").unwrap();
        sh(&wt, &["add", "-A"]);
        sh(&wt, &["commit", "-qm", "job edits a"]);
        fs::write(repo.join("a.txt"), "main version\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "main edits a"]);
        let prepared = prepare(&repo, &mut state, "cp").unwrap();
        assert_eq!(prepared, Prepared::Conflicts(vec!["a.txt".into()]));
        assert_eq!(state.conflicts, vec!["a.txt".to_string()]);
        assert!(merge_in_progress(&wt));
        // Markers still there: prepare reports the conflict again, no commit.
        assert_eq!(
            prepare(&repo, &mut state, "cp").unwrap(),
            Prepared::Conflicts(vec!["a.txt".into()])
        );
        // The resolution turn edits the file; prepare completes the merge.
        fs::write(wt.join("a.txt"), "both versions, reconciled\n").unwrap();
        assert_eq!(
            prepare(&repo, &mut state, "cp").unwrap(),
            Prepared::Ready { new_commit: true }
        );
        assert!(!merge_in_progress(&wt));
        assert!(state.conflicts.is_empty());
        assert!(land(&repo, &state).unwrap());
        assert_eq!(
            fs::read_to_string(repo.join("a.txt")).unwrap(),
            "both versions, reconciled\n"
        );
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_dirty_checkout_on_touched_files_refuses_to_land() {
        let (repo, wt, mut state) = scenario("dirty");
        fs::write(wt.join("a.txt"), "job version\n").unwrap();
        sh(&wt, &["add", "-A"]);
        sh(&wt, &["commit", "-qm", "job edits a"]);
        assert!(matches!(
            prepare(&repo, &mut state, "cp").unwrap(),
            Prepared::Ready { .. }
        ));
        // The user has uncommitted edits to the same file in the main checkout.
        fs::write(repo.join("a.txt"), "my local wip\n").unwrap();
        let err = land(&repo, &state).unwrap_err();
        assert!(
            err.0.starts_with("The base branch cannot be updated:"),
            "{err}"
        );
        assert_eq!(
            fs::read_to_string(repo.join("a.txt")).unwrap(),
            "my local wip\n",
            "edits intact"
        );
        assert_ne!(
            base_tip(&repo, "main").unwrap(),
            state.synced.as_ref().unwrap().head
        );
        // Cleanup refuses while the branch has not landed.
        assert!(cleanup_landed(&repo, &state).is_err());
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn landing_without_a_checkout_updates_the_ref_directly() {
        let (repo, wt, mut state) = scenario("bare-ish");
        // Move the main checkout to another branch so `main` is checked out nowhere.
        sh(&repo, &["checkout", "-q", "-b", "elsewhere"]);
        fs::write(wt.join("job.txt"), "work\n").unwrap();
        assert!(matches!(
            prepare(&repo, &mut state, "cp").unwrap(),
            Prepared::Ready { .. }
        ));
        assert_eq!(branch_checkout(&repo, "main", Some(&wt)).unwrap(), None);
        assert!(land(&repo, &state).unwrap());
        assert_eq!(
            base_tip(&repo, "main").unwrap(),
            state.synced.as_ref().unwrap().head
        );
        assert!(
            !repo.join("job.txt").exists(),
            "the other branch's checkout is untouched"
        );
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn marker_detection_and_locks() {
        let dir = std::env::temp_dir().join(format!(
            "ac-landing-markers-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("x.txt"),
            "<<<<<<< HEAD\na\n=======\nb\n>>>>>>> base\n",
        )
        .unwrap();
        fs::write(dir.join("y.txt"), "clean\n").unwrap();
        let files = vec![
            "x.txt".to_string(),
            "y.txt".to_string(),
            "missing.txt".to_string(),
        ];
        assert_eq!(
            conflict_marker_files(&dir, &files),
            vec!["x.txt".to_string()]
        );
        let a = lock("/p");
        let b = lock("/p");
        assert!(Arc::ptr_eq(&a, &b));
        assert!(!Arc::ptr_eq(&a, &lock("/q")));
        let _ = fs::remove_dir_all(&dir);
    }
}
