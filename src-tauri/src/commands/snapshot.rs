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

#[tauri::command]
pub fn snapshot_restore(commit_sha: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = repo(&state)?;
    snapshot_service::restore(&repo, &commit_sha)
}

#[tauri::command]
pub fn snapshot_delete(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = repo(&state)?;
    snapshot_service::delete(&repo, &id)
}
