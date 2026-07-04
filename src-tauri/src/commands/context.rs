use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};
use crate::services::context_service::{self, ContextStatus, FileStat};
use crate::services::memory_service::{self, MemoryEntry};
use crate::state::AppState;

fn project_root(state: &AppState) -> Option<std::path::PathBuf> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.clone())
}

#[tauri::command]
pub fn context_status(state: State<'_, AppState>) -> AppResult<ContextStatus> {
    context_service::status(project_root(&state).as_deref())
}

#[tauri::command]
pub fn context_read_md(state: State<'_, AppState>, scope: String) -> AppResult<String> {
    context_service::read_md(project_root(&state).as_deref(), &scope)
}

#[tauri::command]
pub fn context_write_md(
    state: State<'_, AppState>,
    scope: String,
    content: String,
    expected_mtime_ms: Option<i64>,
) -> AppResult<FileStat> {
    context_service::write_md(
        project_root(&state).as_deref(),
        &scope,
        &content,
        expected_mtime_ms,
    )
}

#[tauri::command]
pub fn context_open_md_externally(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    scope: String,
) -> AppResult<()> {
    let path = context_service::md_path_for(project_root(&state).as_deref(), &scope)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::Other(format!("open failed: {e}")))
}

#[tauri::command]
pub fn context_generate_starter(state: State<'_, AppState>) -> AppResult<String> {
    let root =
        project_root(&state).ok_or_else(|| AppError::InvalidArgument("no project open".into()))?;
    context_service::generate_starter(&root)
}

#[tauri::command]
pub fn memory_list(state: State<'_, AppState>) -> AppResult<Vec<MemoryEntry>> {
    match project_root(&state) {
        Some(root) => memory_service::list(&root),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
pub fn memory_read(state: State<'_, AppState>, name: String) -> AppResult<String> {
    let root =
        project_root(&state).ok_or_else(|| AppError::InvalidArgument("no project open".into()))?;
    memory_service::read(&root, &name)
}

#[tauri::command]
pub fn memory_delete(state: State<'_, AppState>, name: String) -> AppResult<()> {
    let root =
        project_root(&state).ok_or_else(|| AppError::InvalidArgument("no project open".into()))?;
    memory_service::delete(&root, &name)
}
