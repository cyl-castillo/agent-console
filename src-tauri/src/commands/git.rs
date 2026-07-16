use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::git_service::{self, BranchInfo, GitCommitInfo, GitStatus};
use crate::state::AppState;

/// The checkout git commands operate on: the active session's isolated
/// worktree when one is active (set via `set_active_repo`), else the project
/// root. This is what makes the Changes view follow the active session.
fn current_repo(state: &AppState) -> AppResult<std::path::PathBuf> {
    let s = state.inner.lock();
    if let Some(wt) = &s.active_repo {
        return Ok(wt.clone());
    }
    s.project
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

/// Turns closed more than this long ago don't stamp commits — the trailer is
/// for "commit the work the agent just did", not archaeology.
const TESTIGO_TRAILER_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;

#[tauri::command]
pub fn git_commit(message: String, state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    // Testigo trailer: stamp the commit with the case whose recorded turn
    // produced the staged files (ledger evidence, not active-session
    // guessing). Best-effort — a ledger miss never blocks the commit.
    let mut message = message;
    if !message.contains("Testigo-Case:") {
        let project = state.inner.lock().project.clone();
        if let Some(p) = project {
            let staged = git_service::staged_files(&repo).unwrap_or_default();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let root = p.root.to_string_lossy();
            if let Ok(Some(case)) =
                state
                    .testigo
                    .case_for_files(root.as_ref(), &staged, now, TESTIGO_TRAILER_MAX_AGE_MS)
            {
                message = format!("{}\n\nTestigo-Case: {case}", message.trim_end());
                // V2-A: carry the ledger head into pushed history — the
                // distributed half of the anchor (the local half lives in
                // refs/agent-console/testigo-head).
                if let Ok(Some((seq, hash))) = state.testigo.head(root.as_ref()) {
                    message = format!("{message}\nTestigo-Head: {seq}:{hash}");
                }
            }
        }
    }
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
pub fn git_recent_messages(
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
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
