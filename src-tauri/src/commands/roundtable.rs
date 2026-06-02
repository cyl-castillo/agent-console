use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::services::roundtable_service::RoundtableConfig;
use crate::state::AppState;

#[tauri::command]
pub fn roundtable_start(
    app: AppHandle,
    state: State<'_, AppState>,
    config: RoundtableConfig,
) -> AppResult<String> {
    let repo = state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::Other("no project open".into()))?;
    state.roundtable.start(app, repo, config)
}

#[tauri::command]
pub fn roundtable_pause(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.pause(&id)
}

#[tauri::command]
pub fn roundtable_resume(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.resume(&id)
}

#[tauri::command]
pub fn roundtable_inject(state: State<'_, AppState>, id: String, message: String) -> AppResult<()> {
    state.roundtable.inject(&id, message)
}

#[tauri::command]
pub fn roundtable_stop(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.stop(&id)
}

#[tauri::command]
pub fn roundtable_side_diff(state: State<'_, AppState>, id: String, side: String) -> AppResult<String> {
    state.roundtable.side_diff(&id, &side)
}

#[tauri::command]
pub fn roundtable_apply(
    state: State<'_, AppState>,
    id: String,
    side: String,
) -> AppResult<Option<String>> {
    state.roundtable.apply(&id, &side)
}

#[tauri::command]
pub fn roundtable_discard(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.discard(&id)
}
