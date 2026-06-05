use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::services::roundtable_service::{PersistedRoom, RoomSummary, RoundtableConfig};
use crate::state::AppState;

/// The open project's root, or an error if none is open. Persisted rooms are
/// keyed by it, exactly as `roundtable_start` keys the live run.
fn project_root(state: &State<'_, AppState>) -> AppResult<String> {
    state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.display().to_string())
        .ok_or_else(|| AppError::Other("no project open".into()))
}

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
pub fn roundtable_inject(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    message: String,
) -> AppResult<()> {
    state.roundtable.inject(&app, &id, message)
}

#[tauri::command]
pub fn roundtable_continue(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    extra: u32,
) -> AppResult<()> {
    state.roundtable.continue_run(&app, &id, extra)
}

#[tauri::command]
pub fn roundtable_stop(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.stop(&id)
}

#[tauri::command]
pub fn roundtable_discard(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.discard(&id)
}

/// Persisted rooms for the open project (lightweight, for the sidebar list).
#[tauri::command]
pub fn roundtable_list_rooms(state: State<'_, AppState>) -> AppResult<Vec<RoomSummary>> {
    let root = project_root(&state)?;
    state.roundtable.rooms().summaries(&root)
}

/// Full saved state of one room, for read-only re-hydration.
#[tauri::command]
pub fn roundtable_get_room(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<PersistedRoom>> {
    let root = project_root(&state)?;
    state.roundtable.rooms().get(&root, &id)
}

/// Drop a saved room from this project's history. Idempotent.
#[tauri::command]
pub fn roundtable_delete_room(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let root = project_root(&state)?;
    state.roundtable.rooms().delete_room(&root, &id)
}

/// Rebuild a live run from a saved room so it can be continued (Fase B). Returns
/// the (unchanged) room id, now registered as a live run in the "awaiting" state.
#[tauri::command]
pub fn roundtable_resume_room(state: State<'_, AppState>, id: String) -> AppResult<String> {
    let root = project_root(&state)?;
    let room: PersistedRoom = state
        .roundtable
        .rooms()
        .get(&root, &id)?
        .ok_or_else(|| AppError::NotFound(format!("saved room {id}")))?;
    state.roundtable.restore(PathBuf::from(&root), room)
}
