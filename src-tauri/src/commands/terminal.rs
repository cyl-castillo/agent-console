use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn term_spawn(
    cwd: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    state.terminals.spawn(app, &PathBuf::from(cwd))
}

#[tauri::command]
pub fn term_write(id: String, data: String, state: State<'_, AppState>) -> AppResult<()> {
    state.terminals.write(&id, data.as_bytes())
}

#[tauri::command]
pub fn term_resize(
    id: String,
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.terminals.resize(&id, cols, rows)
}

#[tauri::command]
pub fn term_kill(id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.terminals.kill(&id)
}
