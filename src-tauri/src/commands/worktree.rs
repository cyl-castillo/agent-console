use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::worktree_service::{
    self, MergeOutcome, SetupConfig, WorktreeEntry, WorktreeInfo, WorktreeStatus,
};
use crate::state::AppState;

fn repo(state: &AppState) -> AppResult<PathBuf> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))
}

/// Managed checkout location: a stable per-repo dir under the cache (NOT /tmp —
/// installs done by the setup command must survive reboots). The path hash
/// disambiguates same-named repos in different locations.
fn worktree_root(repo: &std::path::Path) -> AppResult<PathBuf> {
    let base = dirs::cache_dir().ok_or_else(|| AppError::Other("no cache dir".into()))?;
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".into());
    let mut h = DefaultHasher::new();
    repo.to_string_lossy().hash(&mut h);
    Ok(base
        .join("agent-console")
        .join("worktrees")
        .join(format!("{name}-{:08x}", (h.finish() as u32))))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCreated {
    pub info: WorktreeInfo,
    /// Untracked files copied in from the main checkout (.env etc.).
    pub copied: Vec<String>,
    /// Configured install command, if any. The UI runs it in the session
    /// terminal so the user sees it — the backend never runs it implicitly.
    pub setup_command: Option<String>,
}

/// Create an isolated worktree for a new session: branch `agent/<name>` off
/// `base` (default: the branch the main checkout is on), plus workspace setup.
#[tauri::command]
pub fn worktree_create(
    name: String,
    base: Option<String>,
    branch: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<WorktreeCreated> {
    let repo = repo(&state)?;
    // `name` drives the worktree folder (a flat slug); `branch` is the git
    // branch. An explicit branch is used verbatim (the caller's/team's
    // convention, slashes preserved) — we only fall back to the imposed
    // `agent/<slug>` when no branch is given (the legacy SessionList flow).
    let slug = worktree_service::sanitize_branch_component(&name);
    let branch = match branch {
        Some(b) if !b.trim().is_empty() => worktree_service::sanitize_branch_ref(&b),
        _ => format!("agent/{slug}"),
    };
    let dest = worktree_root(&repo)?.join(&slug);
    if dest.exists() {
        return Err(AppError::InvalidArgument(format!(
            "a worktree for '{slug}' already exists — pick another name"
        )));
    }
    let info = worktree_service::create(&repo, &dest, &branch, base.as_deref())?;
    let config = worktree_service::load_setup_config(&repo);
    let copied = worktree_service::copy_setup_files(&repo, &dest, &config);
    Ok(WorktreeCreated {
        info,
        copied,
        setup_command: config.setup,
    })
}

/// The branch name to propose for a ticket worktree — from a skill's
/// `branch-template`, else the project's worktree-setup.json, else "{key}".
/// The UI shows it pre-filled and editable; nothing is imposed.
#[tauri::command]
pub fn worktree_suggest_branch(
    key: String,
    summary: String,
    issue_type: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let repo = repo(&state)?;
    Ok(worktree_service::suggest_branch(&repo, &key, &summary, &issue_type))
}

#[tauri::command]
pub fn worktree_status(
    path: String,
    branch: String,
    base_branch: String,
    state: State<'_, AppState>,
) -> AppResult<WorktreeStatus> {
    let repo = repo(&state)?;
    worktree_service::status(&repo, &PathBuf::from(path), &branch, &base_branch)
}

/// Merge the session branch back into its base (in the main checkout), then
/// optionally tear the worktree down. A conflicted merge is aborted and
/// reported; nothing is torn down in that case.
#[tauri::command]
pub fn worktree_merge(
    path: String,
    branch: String,
    base_branch: String,
    delete_after: bool,
    state: State<'_, AppState>,
) -> AppResult<MergeOutcome> {
    let repo = repo(&state)?;
    let wt = PathBuf::from(&path);
    let message = format!("Merge session branch '{branch}'");
    let outcome = worktree_service::merge_back(&repo, &wt, &branch, &base_branch, &message)?;
    if outcome.merged && delete_after {
        worktree_service::discard(&repo, &wt, &branch, true)?;
    }
    Ok(outcome)
}

#[tauri::command]
pub fn worktree_discard(
    path: String,
    branch: String,
    delete_branch: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let repo = repo(&state)?;
    worktree_service::discard(&repo, &PathBuf::from(path), &branch, delete_branch)
}

#[tauri::command]
pub fn worktree_list(state: State<'_, AppState>) -> AppResult<Vec<WorktreeEntry>> {
    let repo = repo(&state)?;
    worktree_service::list(&repo)
}

/// Point git/snapshot commands (and the change watcher) at the active
/// session's checkout. `None` or the project root itself clears the override.
/// Anything else must be a registered worktree of the open project — we never
/// let the UI aim git commands at an arbitrary directory.
#[tauri::command]
pub fn set_active_repo(
    path: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let repo = repo(&state)?;
    let target = match path {
        None => None,
        Some(p) if std::path::Path::new(&p) == repo.as_path() => None,
        Some(p) => {
            let pb = PathBuf::from(&p);
            let canon = pb.canonicalize().unwrap_or_else(|_| pb.clone());
            let registered = worktree_service::list(&repo)?.into_iter().any(|e| {
                let ep = PathBuf::from(&e.path);
                ep.canonicalize().unwrap_or(ep) == canon
            });
            if !registered {
                return Err(AppError::InvalidArgument(format!(
                    "'{p}' is not a worktree of the open project"
                )));
            }
            Some(pb)
        }
    };
    let watch_root = target.clone().unwrap_or_else(|| repo.clone());
    state.inner.lock().active_repo = target;
    // Re-aim the debounced fs watcher so `git://changed` events (and the
    // Changes auto-refresh they drive) follow the same checkout.
    state.git_watcher.watch(app, watch_root);
    Ok(())
}

/// Remove managed worktree checkouts that no persisted session references any
/// more (e.g. the app died before a session was cleaned up). Branches are kept
/// — only the checkout dirs go. Returns the removed paths.
#[tauri::command]
pub fn worktree_prune_orphans(
    keep: Vec<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let repo = repo(&state)?;
    let root = worktree_root(&repo)?;
    let root = root.canonicalize().unwrap_or(root);
    let canon = |p: &str| {
        let pb = PathBuf::from(p);
        pb.canonicalize().unwrap_or(pb)
    };
    let keep: Vec<PathBuf> = keep.iter().map(|p| canon(p)).collect();
    let mut removed = Vec::new();
    for entry in worktree_service::list(&repo)? {
        let path = canon(&entry.path);
        if !path.starts_with(&root) || keep.contains(&path) {
            continue;
        }
        if worktree_service::discard(&repo, &path, &entry.branch, false).is_ok() {
            removed.push(entry.path);
        }
    }
    Ok(removed)
}

#[tauri::command]
pub fn worktree_setup_get(state: State<'_, AppState>) -> AppResult<SetupConfig> {
    let repo = repo(&state)?;
    Ok(worktree_service::load_setup_config(&repo))
}

#[tauri::command]
pub fn worktree_setup_set(config: SetupConfig, state: State<'_, AppState>) -> AppResult<()> {
    let repo = repo(&state)?;
    worktree_service::save_setup_config(&repo, &config)
}
