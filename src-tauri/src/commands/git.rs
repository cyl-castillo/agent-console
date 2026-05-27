use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::git_service::{self, BranchInfo, GitCommitInfo, GitStatus};
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

#[tauri::command]
pub fn git_branches(state: State<'_, AppState>) -> AppResult<Vec<BranchInfo>> {
    let repo = current_repo(&state)?;
    git_service::branches(&repo)
}

#[tauri::command]
pub fn git_checkout_branch(name: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = current_repo(&state)?;
    git_service::checkout_branch(&repo, &name)
}

#[tauri::command]
pub fn git_recent_messages(limit: Option<u32>, state: State<'_, AppState>) -> AppResult<Vec<String>> {
    let repo = current_repo(&state)?;
    git_service::recent_messages(&repo, limit.unwrap_or(10))
}

#[tauri::command]
pub fn git_head_message(state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    git_service::head_message(&repo)
}

#[tauri::command]
pub fn git_amend_commit(message: String, state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    git_service::amend_commit(&repo, &message)
}

#[tauri::command]
pub fn git_file_log(
    file: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<Vec<GitCommitInfo>> {
    let repo = current_repo(&state)?;
    git_service::file_log(&repo, &file, limit.unwrap_or(5))
}
