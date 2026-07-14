use tauri::State;

use crate::error::AppResult;
use crate::services::notes_service::Note;
use crate::state::AppState;

#[tauri::command]
pub fn notes_list(project_root: String, state: State<'_, AppState>) -> AppResult<Vec<Note>> {
    state.notes.list(&project_root)
}

#[tauri::command]
pub fn notes_save(
    project_root: String,
    notes: Vec<Note>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.notes.save(&project_root, notes)
}
