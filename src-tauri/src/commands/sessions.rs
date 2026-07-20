use tauri::State;

use crate::error::AppResult;
use crate::services::sessions_service::PersistedSession;
use crate::state::AppState;

#[tauri::command]
pub fn sessions_list(
    project_root: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Vec<PersistedSession>> {
    let (sessions, quarantined) = state.sessions.list_recovering(&project_root)?;
    if let Some(path) = quarantined {
        // Corrupt history was set aside so persistence could resume — say so
        // loudly; this exact failure used to be silent and permanent (#72).
        use tauri::Emitter;
        let _ = app.emit(
            "sessions://quarantined",
            serde_json::json!({ "path": path }),
        );
    }
    Ok(sessions)
}

#[tauri::command]
pub fn sessions_save(
    project_root: String,
    sessions: Vec<PersistedSession>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.sessions.save(&project_root, sessions)
}
