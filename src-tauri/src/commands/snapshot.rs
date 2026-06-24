use std::path::PathBuf;

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::snapshot_service;
use crate::state::AppState;

fn repo(state: &AppState) -> AppResult<PathBuf> {
    state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))
}

/// Restore the working tree to a snapshot. `read-tree --reset -u` is destructive
/// to ALL work done after the target snapshot, not just the turn being undone, so
/// we first capture the current tree as a fresh "pre-restore" snapshot. That makes
/// the restore itself undoable. The returned commit sha (if any) lets the UI offer
/// an "undo last restore". Capturing the backup never blocks the restore.
#[tauri::command]
pub fn snapshot_restore(commit_sha: String, state: State<'_, AppState>) -> AppResult<Option<String>> {
    let repo = repo(&state)?;
    let pre_id = format!("pre-restore-{}", now_nanos());
    let pre = snapshot_service::create(&repo, &pre_id)
        .ok()
        .flatten()
        .map(|s| s.commit_sha);
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

#[tauri::command]
pub fn snapshot_delete(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = repo(&state)?;
    snapshot_service::delete(&repo, &id)
}
