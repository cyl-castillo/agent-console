use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::git_service::{self, GitStatus};
use crate::state::AppState;

fn current_repo(state: &AppState) -> AppResult<std::path::PathBuf> {
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
pub fn git_status(state: State<'_, AppState>) -> AppResult<GitStatus> {
    let repo = current_repo(&state)?;
    git_service::status(&repo)
}

#[tauri::command]
pub fn git_diff_file(file: String, state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    git_service::diff_file(&repo, &file)
}

#[tauri::command]
pub fn git_revert_file(file: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = current_repo(&state)?;
    git_service::revert_file(&repo, &file)
}

#[tauri::command]
pub fn git_stage_file(file: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = current_repo(&state)?;
    git_service::stage_file(&repo, &file)
}

#[tauri::command]
pub fn git_unstage_file(file: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = current_repo(&state)?;
    git_service::unstage_file(&repo, &file)
}

#[tauri::command]
pub fn git_commit(message: String, state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    git_service::commit(&repo, &message)
}
