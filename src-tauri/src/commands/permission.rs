use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn perm_respond(
    id: String,
    allow: bool,
    reason: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.agent.permissions.respond(&id, allow, reason)
}

#[tauri::command]
pub fn perm_set_approve_all(
    enabled: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.agent.permissions.set_approve_all(enabled)
}
