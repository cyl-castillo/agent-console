use tauri::State;

use crate::error::AppResult;
use crate::services::sessions_service::PersistedSession;
use crate::state::AppState;

#[tauri::command]
pub fn sessions_list(
    project_root: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<PersistedSession>> {
    state.sessions.list(&project_root)
}

#[tauri::command]
pub fn sessions_save(
    project_root: String,
    sessions: Vec<PersistedSession>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.sessions.save(&project_root, sessions)
}
