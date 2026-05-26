use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::services::hooks_service::HooksStatus;
use crate::state::AppState;

#[tauri::command]
pub fn hooks_status(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    Ok(state.hooks.status())
}

#[tauri::command]
pub fn hooks_install(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    state.hooks.install()
}

#[tauri::command]
pub fn hooks_uninstall(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    state.hooks.uninstall()
}

#[tauri::command]
pub fn hooks_start_watcher(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.hooks.start_watcher(app);
    Ok(())
}

#[tauri::command]
pub fn approval_respond(
    id: String,
    decision: String,
    reason: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.hooks.respond(&id, &decision, reason.as_deref())
}
