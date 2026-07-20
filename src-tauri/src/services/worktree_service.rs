//! Per-session isolated git worktrees.
//!
//! A session can opt into its own checkout: a fresh `agent/<name>` branch off a
//! base branch, checked out into a managed directory outside the repo. The agent
//! works there without touching the user's checkout; when the session ends the
//! human either merges the branch back into the base or discards it.
//!
//! Design rules:
//! - The user's main checkout is never mutated except by an explicit merge-back,
//!   and that merge only runs when the main checkout is already on the base
//!   branch (we never switch branches behind the user's back).
//! - A failed merge (conflicts) is always aborted so the main checkout is left
//!   exactly as it was; the conflict list is reported instead.
//! - Destructive ops (`discard`) refuse paths that git does not report as a
//!   registered worktree of the repo — we never `remove --force` an arbitrary dir.
//! - Workspace setup: worktrees do not inherit untracked files, so `.env`-style
//!   secrets/config are copied in per project config (`.claude/worktree-setup.json`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::proc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStatus {
    pub dirty_files: usize,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    pub merged: bool,
    pub merge_commit: Option<String>,
    pub conflict_files: Vec<String>,
    pub detail: String,
}

/// Per-project workspace setup: which untracked files to copy into a fresh
/// worktree, and an optional install command the UI can run in the session
/// terminal (visible to the user — this service never runs it implicitly).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SetupConfig {
    pub copy: Vec<String>,
    pub setup: Option<String>,
    /// The team's branch-naming convention for ticket worktrees, e.g.
    /// "feature/{key}" or "{key}-{slug}". Placeholders: {key} {slug} {type}.
    /// None = no project convention (we fall back to a skill's, else "{key}").
    #[serde(default)]
    pub branch_template: Option<String>,
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            copy: vec![".env".into(), ".env.*".into()],
            setup: None,
            branch_template: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeEntry {
    pub path: String,
    pub branch: String,
}

fn git(repo: &Path, args: &[&str]) -> AppResult<std::process::Output> {
    Ok(proc::command("git").args(args).current_dir(repo).output()?)
}

fn git_ok(repo: &Path, args: &[&str], what: &str) -> AppResult<std::process::Output> {
    let out = git(repo, args)?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(AppError::Other(format!("{what}: {msg}")));
    }
    Ok(out)
}

fn stdout_line(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Branch-name slug for the part after `agent/`: keep alphanumerics plus
/// `-_.`, squeeze everything else (including `/`) to `-`. Git rejects some
/// residual shapes (e.g. `..`, trailing `.lock`) — those surface as a clear
/// error from `worktree add` rather than being silently rewritten here.
pub fn sanitize_branch_component(name: &str) -> String {
    let slug: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "session".into()
    } else {
        slug
    }
}

/// The branch the main checkout is currently on. Detached HEAD is an error:
/// merge-back needs a real base branch to return to.
pub fn current_branch(repo: &Path) -> AppResult<String> {
    let out = git_ok(repo, &["branch", "--show-current"], "git branch")?;
    let branch = stdout_line(&out);
    if branch.is_empty() {
        return Err(AppError::InvalidArgument(
            "repo is on a detached HEAD — pick an explicit base branch".into(),
        ));
    }
    Ok(branch)
}

/// Create `<branch>` off `<base>` and check it out at `dest`.
pub fn create(
    repo: &Path,
    dest: &Path,
    branch: &str,
    base: Option<&str>,
) -> AppResult<WorktreeInfo> {
    let base_branch = match base {
        Some(b) if !b.trim().is_empty() => b.trim().to_string(),
        _ => current_branch(repo)?,
    };
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let out = git(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            &dest.to_string_lossy(),
            &base_branch,
        ],
    )?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(AppError::Other(format!(
            "git worktree add (needs a repo with at least one commit): {msg}"
        )));
    }
    Ok(WorktreeInfo {
        path: dest.to_string_lossy().to_string(),
        branch: branch.to_string(),
        base_branch,
    })
}

/// Copy configured untracked files (e.g. `.env`) from the main checkout into
/// the worktree. Patterns are paths relative to the repo root; `*` globs within
/// the final path component only (`.env.*`, `config/*.local`). Files that
/// already exist in the worktree (i.e. tracked files) are left alone.
pub fn copy_setup_files(repo: &Path, worktree: &Path, config: &SetupConfig) -> Vec<String> {
    let mut copied = Vec::new();
    for pattern in &config.copy {
        let rel = Path::new(pattern);
        // Refuse absolute patterns and parent-dir escapes outright.
        if rel.is_absolute() || pattern.contains("..") {
            continue;
        }
        let (dir_rel, name_pat) = match (rel.parent(), rel.file_name()) {
            (Some(d), Some(f)) => (d.to_path_buf(), f.to_string_lossy().to_string()),
            _ => continue,
        };
        let src_dir = repo.join(&dir_rel);
        let entries = match fs::read_dir(&src_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if !glob_match(&name_pat, &fname) {
                continue;
            }
            if !entry.path().is_file() {
                continue;
            }
            let rel_path = dir_rel.join(&fname);
            let target = worktree.join(&rel_path);
            if target.exists() {
                continue;
            }
            if let Some(parent) = target.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if fs::copy(entry.path(), &target).is_ok() {
                copied.push(rel_path.to_string_lossy().to_string());
            }
        }
    }
    copied.sort();
    copied
}

/// `*` matches any run of characters (no path separators involved — this is
/// matched against a single file name); everything else is literal.
fn glob_match(pattern: &str, name: &str) -> bool {
    fn rec(p: &[u8], n: &[u8]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some(b'*'), _) => rec(&p[1..], n) || (!n.is_empty() && rec(p, &n[1..])),
            (Some(pc), Some(nc)) if pc == nc => rec(&p[1..], &n[1..]),
            _ => false,
        }
    }
    rec(pattern.as_bytes(), name.as_bytes())
}

/// Dirty file count in the worktree + ahead/behind of the branch vs its base.
pub fn status(
    repo: &Path,
    worktree: &Path,
    branch: &str,
    base_branch: &str,
) -> AppResult<WorktreeStatus> {
    let st = git_ok(worktree, &["status", "--porcelain"], "git status")?;
    let dirty_files = String::from_utf8_lossy(&st.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    let range = format!("{base_branch}...{branch}");
    let out = git_ok(
        repo,
        &["rev-list", "--left-right", "--count", &range],
        "git rev-list",
    )?;
    let counts = stdout_line(&out);
    let mut it = counts.split_whitespace();
    let behind = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ahead = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok(WorktreeStatus {
        dirty_files,
        ahead,
        behind,
    })
}

/// Commit everything outstanding in the worktree as one commit. Returns whether
/// a commit was created (false = tree was clean). Uses the user's own git
/// identity, inherited from repo/global config.
pub fn commit_all(worktree: &Path, message: &str) -> AppResult<bool> {
    git_ok(worktree, &["add", "-A"], "git add")?;
    // `diff --cached --quiet` exits non-zero exactly when something is staged.
    let staged = git(worktree, &["diff", "--cached", "--quiet"])?;
    if staged.status.success() {
        return Ok(false);
    }
    git_ok(worktree, &["commit", "-q", "-m", message], "git commit")?;
    Ok(true)
}

/// Merge the session branch back into its base, in the MAIN checkout.
///
/// Outstanding work in the worktree is auto-committed first so nothing is
/// silently dropped. The main checkout must already be on the base branch;
/// conflicts abort the merge and leave the main checkout untouched.
pub fn merge_back(
    repo: &Path,
    worktree: &Path,
    branch: &str,
    base_branch: &str,
    message: &str,
) -> AppResult<MergeOutcome> {
    commit_all(worktree, &format!("session work on {branch}"))?;

    let on = current_branch(repo)?;
    if on != base_branch {
        return Err(AppError::InvalidArgument(format!(
            "main checkout is on '{on}', not the base branch '{base_branch}' — switch back first (nothing was merged)"
        )));
    }

    let out = git(repo, &["merge", "--no-ff", branch, "-m", message])?;
    if out.status.success() {
        let head = git_ok(repo, &["rev-parse", "HEAD"], "git rev-parse")?;
        return Ok(MergeOutcome {
            merged: true,
            merge_commit: Some(stdout_line(&head)),
            conflict_files: Vec::new(),
            detail: String::new(),
        });
    }

    // Collect conflicts (if any), then always abort so the checkout is clean.
    let conflicts = git(repo, &["diff", "--name-only", "--diff-filter=U"])
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.to_string())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let _ = git(repo, &["merge", "--abort"]);
    let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let detail = if detail.is_empty() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        detail
    };
    Ok(MergeOutcome {
        merged: false,
        merge_commit: None,
        conflict_files: conflicts,
        detail,
    })
}

/// Registered worktrees of the repo (excluding the main checkout itself).
pub fn list(repo: &Path) -> AppResult<Vec<WorktreeEntry>> {
    let out = git_ok(
        repo,
        &["worktree", "list", "--porcelain"],
        "git worktree list",
    )?;
    let raw = String::from_utf8_lossy(&out.stdout);
    let mut entries = Vec::new();
    let mut path: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut first = true;
    for line in raw.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let (Some(p), Some(b)) = (path.take(), branch.take()) {
                if first {
                    first = false;
                } else {
                    entries.push(WorktreeEntry { path: p, branch: b });
                }
            } else if path.take().is_some() && first {
                first = false;
            }
            continue;
        }
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(p.to_string());
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            branch = Some(b.to_string());
        }
    }
    Ok(entries)
}

fn assert_registered(repo: &Path, worktree: &Path) -> AppResult<()> {
    let wt = worktree
        .canonicalize()
        .unwrap_or_else(|_| worktree.to_path_buf());
    let registered = list(repo)?.into_iter().any(|e| {
        let p = PathBuf::from(&e.path);
        p.canonicalize().unwrap_or(p) == wt
    });
    if !registered {
        return Err(AppError::InvalidArgument(format!(
            "'{}' is not a registered worktree of this repo",
            worktree.display()
        )));
    }
    Ok(())
}

/// Tear down a session worktree. The checkout dir is removed; the branch (and
/// its commits) survives unless `delete_branch` is set. Refuses paths git does
/// not know as worktrees of this repo.
pub fn discard(repo: &Path, worktree: &Path, branch: &str, delete_branch: bool) -> AppResult<()> {
    assert_registered(repo, worktree)?;
    git_ok(
        repo,
        &["worktree", "remove", "--force", &worktree.to_string_lossy()],
        "git worktree remove",
    )?;
    let _ = git(repo, &["worktree", "prune"]);
    if delete_branch {
        git_ok(repo, &["branch", "-D", branch], "git branch -D")?;
    }
    Ok(())
}

// ----- Per-project setup config (.claude/worktree-setup.json) -----

fn setup_config_path(repo: &Path) -> PathBuf {
    repo.join(".claude").join("worktree-setup.json")
}

/// Missing or unreadable config falls back to the default (copy `.env*`, no
/// setup command) — a broken JSON file must not block creating sessions.
/// Sanitize a full branch ref, preserving '/' separators (so "feature/ABC-123"
/// survives) while cleaning each segment with `sanitize_branch_component`.
/// Empty → "session".
pub fn sanitize_branch_ref(name: &str) -> String {
    let cleaned = name
        .trim()
        .trim_matches('/')
        .split('/')
        .map(sanitize_branch_component)
        .filter(|s| !s.is_empty() && s != "session")
        .collect::<Vec<_>>()
        .join("/");
    if cleaned.is_empty() {
        "session".to_string()
    } else {
        cleaned
    }
}

/// Turn a summary into a short kebab slug for a branch name.
fn summary_slug(summary: &str) -> String {
    let words: Vec<String> = summary
        .split_whitespace()
        .take(6)
        .map(|w| {
            w.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    words.join("-")
}

/// Interpolate a branch template's placeholders. {key} {slug} {type}.
pub fn apply_branch_template(template: &str, key: &str, summary: &str, issue_type: &str) -> String {
    template
        .replace("{key}", key)
        .replace("{slug}", &summary_slug(summary))
        .replace("{type}", &issue_type.to_lowercase())
}

/// The branch-naming convention to propose, in priority order:
/// 1. a skill declaring `branch-template:` in its frontmatter (team convention
///    encoded as a skill; project skills before user skills),
/// 2. the project's `.claude/worktree-setup.json` `branchTemplate`,
/// 3. None — the caller defaults to just the ticket key.
pub fn resolve_branch_template(repo: &Path) -> Option<String> {
    if let Ok(skills) = crate::services::skills_service::list(Some(repo)) {
        // Project skills win over user skills; list() returns project first.
        for skill in &skills {
            if let Some(t) = skill_branch_template(&skill.path) {
                return Some(t);
            }
        }
    }
    load_setup_config(repo).branch_template
}

/// Read a `branch-template:` value from a SKILL.md's YAML frontmatter, if any.
fn skill_branch_template(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    if !content.starts_with("---") {
        return None;
    }
    let after = &content[3..];
    let end = after.find("\n---")?;
    for line in after[..end].lines() {
        if let Some(rest) = line.trim().strip_prefix("branch-template:") {
            let v = rest.trim().trim_matches(['"', '\'']).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// The full proposed branch name for a ticket: resolved template (or "{key}"),
/// interpolated and sanitized. The UI shows this pre-filled and editable.
pub fn suggest_branch(repo: &Path, key: &str, summary: &str, issue_type: &str) -> String {
    let template = resolve_branch_template(repo).unwrap_or_else(|| "{key}".to_string());
    sanitize_branch_ref(&apply_branch_template(&template, key, summary, issue_type))
}

pub fn load_setup_config(repo: &Path) -> SetupConfig {
    let path = setup_config_path(repo);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => SetupConfig::default(),
    }
}

pub fn save_setup_config(repo: &Path, config: &SetupConfig) -> AppResult<()> {
    let path = setup_config_path(repo);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(config)
        .map_err(|e| AppError::Other(format!("serialize worktree setup: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn run(args: &[&str], cwd: &Path) -> std::process::Output {
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
        let repo = std::env::temp_dir().join(format!("ac-wt-{tag}-{nanos}"));
        fs::create_dir_all(&repo).unwrap();
        run(&["init", "-q", "-b", "main"], &repo);
        run(&["config", "user.email", "t@t"], &repo);
        run(&["config", "user.name", "T"], &repo);
        run(&["config", "commit.gpgsign", "false"], &repo);
        fs::write(repo.join("a.txt"), "base").unwrap();
        run(&["add", "-A"], &repo);
        run(&["commit", "-qm", "seed"], &repo);
        repo
    }

    fn wt_dest(repo: &Path, name: &str) -> PathBuf {
        // Sibling of the repo, never inside it.
        repo.parent().unwrap().join(format!(
            "{}-wt-{name}",
            repo.file_name().unwrap().to_string_lossy()
        ))
    }

    fn cleanup(paths: &[&Path]) {
        for p in paths {
            let _ = fs::remove_dir_all(p);
        }
    }

    #[test]
    fn create_copies_env_and_tracks_ahead() {
        let repo = init_repo("create");
        fs::write(repo.join(".env"), "SECRET=1").unwrap();
        fs::write(repo.join(".env.local"), "LOCAL=1").unwrap();

        let dest = wt_dest(&repo, "s1");
        let info = create(&repo, &dest, "agent/s1", None).unwrap();
        assert_eq!(info.base_branch, "main");
        assert!(dest.join("a.txt").exists(), "tracked file is checked out");

        let copied = copy_setup_files(&repo, &dest, &SetupConfig::default());
        assert_eq!(copied, vec![".env".to_string(), ".env.local".to_string()]);
        assert_eq!(fs::read_to_string(dest.join(".env")).unwrap(), "SECRET=1");

        // Untouched worktree: clean and even with base.
        let st = status(&repo, &dest, "agent/s1", "main").unwrap();
        assert_eq!((st.dirty_files, st.ahead, st.behind), (2, 0, 0)); // the 2 copied .env files are untracked

        // One committed edit → ahead 1, clean.
        fs::write(dest.join("a.txt"), "edited").unwrap();
        assert!(commit_all(&dest, "work").unwrap());
        assert!(
            !commit_all(&dest, "noop").unwrap(),
            "clean tree → no commit"
        );
        let st = status(&repo, &dest, "agent/s1", "main").unwrap();
        assert_eq!((st.ahead, st.behind), (1, 0));

        // Registered in list(), main checkout excluded.
        let entries = list(&repo).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].branch, "agent/s1");

        cleanup(&[&dest, &repo]);
    }

    #[test]
    fn merge_back_lands_work_and_discard_cleans_up() {
        let repo = init_repo("merge");
        let dest = wt_dest(&repo, "s2");
        create(&repo, &dest, "agent/s2", Some("main")).unwrap();

        // Uncommitted work in the worktree must be auto-committed and merged.
        fs::write(dest.join("feature.txt"), "done").unwrap();
        let outcome = merge_back(&repo, &dest, "agent/s2", "main", "Merge session s2").unwrap();
        assert!(outcome.merged, "detail: {}", outcome.detail);
        assert!(outcome.merge_commit.is_some());
        assert_eq!(
            fs::read_to_string(repo.join("feature.txt")).unwrap(),
            "done"
        );

        discard(&repo, &dest, "agent/s2", true).unwrap();
        assert!(!dest.exists(), "checkout dir removed");
        let br = run(&["branch", "--list", "agent/s2"], &repo);
        assert_eq!(
            String::from_utf8_lossy(&br.stdout).trim(),
            "",
            "branch deleted"
        );

        cleanup(&[&repo]);
    }

    #[test]
    fn merge_conflict_aborts_and_leaves_main_clean() {
        let repo = init_repo("conflict");
        let dest = wt_dest(&repo, "s3");
        create(&repo, &dest, "agent/s3", None).unwrap();

        // Divergent edits to the same file on both sides.
        fs::write(dest.join("a.txt"), "from-session").unwrap();
        commit_all(&dest, "session edit").unwrap();
        fs::write(repo.join("a.txt"), "from-main").unwrap();
        run(&["commit", "-aqm", "main edit"], &repo);

        let outcome = merge_back(&repo, &dest, "agent/s3", "main", "Merge session s3").unwrap();
        assert!(!outcome.merged);
        assert_eq!(outcome.conflict_files, vec!["a.txt".to_string()]);

        // Aborted: no in-progress merge, tree back to the main-side content.
        assert!(
            !repo.join(".git/MERGE_HEAD").exists(),
            "merge fully aborted"
        );
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "from-main");
        let st = run(&["status", "--porcelain"], &repo);
        assert_eq!(
            String::from_utf8_lossy(&st.stdout).trim(),
            "",
            "main checkout clean"
        );

        discard(&repo, &dest, "agent/s3", true).unwrap();
        cleanup(&[&repo]);
    }

    #[test]
    fn merge_refuses_when_main_is_elsewhere() {
        let repo = init_repo("elsewhere");
        let dest = wt_dest(&repo, "s4");
        create(&repo, &dest, "agent/s4", Some("main")).unwrap();
        run(&["checkout", "-qb", "other"], &repo);

        let err = merge_back(&repo, &dest, "agent/s4", "main", "m").unwrap_err();
        assert!(
            err.to_string().contains("not the base branch"),
            "got: {err}"
        );

        run(&["checkout", "-q", "main"], &repo);
        discard(&repo, &dest, "agent/s4", true).unwrap();
        cleanup(&[&repo]);
    }

    #[test]
    fn discard_refuses_unregistered_paths() {
        let repo = init_repo("unreg");
        let stranger = repo.parent().unwrap().join("ac-wt-stranger-dir");
        fs::create_dir_all(&stranger).unwrap();
        let err = discard(&repo, &stranger, "agent/x", false).unwrap_err();
        assert!(
            err.to_string().contains("not a registered worktree"),
            "got: {err}"
        );
        assert!(stranger.exists(), "stranger dir untouched");
        cleanup(&[&stranger, &repo]);
    }

    #[test]
    fn setup_config_round_trip_and_fallbacks() {
        let repo = init_repo("cfg");
        // Missing file → defaults.
        assert_eq!(load_setup_config(&repo), SetupConfig::default());
        // Corrupt file → defaults, not an error.
        fs::create_dir_all(repo.join(".claude")).unwrap();
        fs::write(setup_config_path(&repo), "{not json").unwrap();
        assert_eq!(load_setup_config(&repo), SetupConfig::default());
        // Round trip.
        let cfg = SetupConfig {
            copy: vec![".env".into(), "config/*.local".into()],
            setup: Some("npm ci".into()),
            ..Default::default()
        };
        save_setup_config(&repo, &cfg).unwrap();
        assert_eq!(load_setup_config(&repo), cfg);
        cleanup(&[&repo]);
    }

    #[test]
    fn copy_patterns_stay_inside_the_repo_and_glob_within_a_dir() {
        let repo = init_repo("glob");
        fs::create_dir_all(repo.join("config")).unwrap();
        fs::write(repo.join("config/db.local"), "x").unwrap();
        fs::write(repo.join("config/db.prod"), "x").unwrap();
        fs::write(repo.parent().unwrap().join("outside.local"), "x").unwrap();

        let dest = wt_dest(&repo, "s5");
        create(&repo, &dest, "agent/s5", None).unwrap();
        let cfg = SetupConfig {
            copy: vec![
                "config/*.local".into(),
                "../outside.local".into(), // escape attempt → ignored
                "/etc/passwd".into(),      // absolute → ignored
            ],
            setup: None,
            ..Default::default()
        };
        let copied = copy_setup_files(&repo, &dest, &cfg);
        assert_eq!(copied, vec!["config/db.local".to_string()]);
        assert!(!dest.join("config/db.prod").exists());

        discard(&repo, &dest, "agent/s5", true).unwrap();
        let _ = fs::remove_file(repo.parent().unwrap().join("outside.local"));
        cleanup(&[&repo]);
    }

    #[test]
    fn glob_match_basics() {
        assert!(glob_match(".env", ".env"));
        assert!(glob_match(".env.*", ".env.local"));
        assert!(!glob_match(".env.*", ".env"));
        assert!(glob_match("*.local", "db.local"));
        assert!(!glob_match("*.local", "db.localx"));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("a*b", "acx"));
        assert!(glob_match("a*b", "ab"));
    }

    #[test]
    fn sanitize_branch_component_cases() {
        assert_eq!(
            sanitize_branch_component("Fix login flow"),
            "Fix-login-flow"
        );
        assert_eq!(sanitize_branch_component("  weird//name?! "), "weird--name");
        assert_eq!(sanitize_branch_component("---"), "session");
        assert_eq!(sanitize_branch_component(""), "session");
    }

    #[test]
    fn sanitize_branch_ref_preserves_slashes() {
        assert_eq!(sanitize_branch_ref("feature/ABC-123"), "feature/ABC-123");
        assert_eq!(
            sanitize_branch_ref("feature/fix login"),
            "feature/fix-login"
        );
        assert_eq!(
            sanitize_branch_ref("/leading/trailing/"),
            "leading/trailing"
        );
        assert_eq!(sanitize_branch_ref("bad??/name!!"), "bad/name");
        assert_eq!(sanitize_branch_ref(""), "session");
    }

    #[test]
    fn apply_branch_template_fills_placeholders() {
        assert_eq!(
            apply_branch_template(
                "feature/{key}",
                "ABC-123",
                "Fix the login redirect",
                "Story"
            ),
            "feature/ABC-123"
        );
        // {slug} takes up to the first 6 summary words.
        assert_eq!(
            apply_branch_template(
                "{key}-{slug}",
                "ABC-1",
                "Fix the login redirect bug now please",
                "Bug"
            ),
            "ABC-1-fix-the-login-redirect-bug-now"
        );
        assert_eq!(
            apply_branch_template("{type}/{key}", "ABC-1", "x", "Bug"),
            "bug/ABC-1"
        );
    }

    #[test]
    fn suggest_branch_defaults_to_key_without_convention() {
        // No skills, no setup config → template defaults to "{key}".
        let repo = init_repo("br-default");
        assert_eq!(
            suggest_branch(&repo, "PROJ-42", "Whatever summary", "Task"),
            "PROJ-42"
        );
        cleanup(&[&repo]);
    }

    #[test]
    fn suggest_branch_uses_project_setup_template() {
        let repo = init_repo("br-config");
        save_setup_config(
            &repo,
            &SetupConfig {
                branch_template: Some("feature/{key}-{slug}".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            suggest_branch(&repo, "ABC-9", "Add dark mode toggle", "Story"),
            "feature/ABC-9-add-dark-mode-toggle"
        );
        cleanup(&[&repo]);
    }
}
