use std::path::PathBuf;

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::snapshot_service;
use crate::state::AppState;

/// Snapshots are taken/restored in the checkout the active session runs in
/// (its isolated worktree when set, else the project root) — restoring a
/// worktree-session snapshot into the main checkout would wreck the wrong tree.
fn repo(state: &AppState) -> AppResult<PathBuf> {
    let s = state.inner.lock();
    if let Some(wt) = &s.active_repo {
        return Ok(wt.clone());
    }
    s.project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))
}

/// Restore the working tree to a snapshot. `read-tree --reset -u` is destructive
/// to ALL work done after the target snapshot, not just the turn being undone, so
/// we first capture the current tree as a fresh "pre-restore" snapshot. That makes
/// the restore itself undoable. The returned commit sha (if any) lets the UI offer
/// an "undo last restore". If the backup cannot be taken the restore does not
/// happen: it would overwrite uncommitted work with no way back.
#[tauri::command(async)]
pub fn snapshot_restore(
    commit_sha: String,
    state: State<'_, AppState>,
) -> AppResult<Option<String>> {
    let repo = repo(&state)?;
    let pre_id = format!("pre-restore-{}", now_nanos());
    let pre = safety_snapshot(&repo, &pre_id)?;
    snapshot_service::restore(&repo, &commit_sha)?;
    Ok(pre)
}

fn now_nanos() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[tauri::command(async)]
pub fn snapshot_delete(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = repo(&state)?;
    snapshot_service::delete(&repo, &id)
}

/// The snapshot taken right before a restore overwrites the tree. `Ok(None)`
/// only when there is no git repo (nothing to restore either); a failure to
/// capture is an error, because proceeding would destroy what it failed to save.
pub(crate) fn safety_snapshot(repo: &std::path::Path, id: &str) -> AppResult<Option<String>> {
    match snapshot_service::create(repo, id) {
        Ok(s) => Ok(s.map(|s| s.commit_sha)),
        Err(e) => Err(crate::error::AppError::Other(format!(
            "Nothing was restored: the safety snapshot of your current files could not be taken ({e})"
        ))),
    }
}
