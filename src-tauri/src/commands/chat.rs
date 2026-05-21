use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::services::snapshot_service::Snapshot;
use crate::state::AppState;

#[tauri::command]
pub fn chat_send(
    text: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<Snapshot>> {
    let cwd = state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))?;
    state.agent.send(app, &cwd, text)
}

#[tauri::command]
pub fn chat_reset(state: State<'_, AppState>) -> AppResult<()> {
    state.agent.reset();
    Ok(())
}
