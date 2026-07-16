use std::path::PathBuf;

use tauri::State;

use crate::error::AppResult;
use crate::services::projects_service::{self, RecentProject};
use crate::state::AppState;

#[tauri::command]
pub fn projects_recent() -> AppResult<Vec<RecentProject>> {
    Ok(projects_service::load())
}

#[tauri::command]
pub fn projects_last() -> AppResult<Option<RecentProject>> {
    Ok(projects_service::last())
}

#[tauri::command]
pub fn projects_forget(path: String, state: State<'_, AppState>) -> AppResult<()> {
    projects_service::forget(&path)?;
    // Forgetting a project also drops its persisted sessions (the scrollback
    // blob is the bulk of sessions.json). Notes and the Testigo ledger stay:
    // notes are cheap and deliberate, and evidence is never deleted implicitly.
    // save() with an empty list removes the project's entry.
    let _ = state.sessions.save(&path, Vec::new());
    Ok(())
}

#[tauri::command]
pub fn projects_remember(path: String) -> AppResult<()> {
    projects_service::remember(&PathBuf::from(path))
}
