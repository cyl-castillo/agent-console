use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::palette_service;
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 10_000;

#[tauri::command]
pub fn palette_index_files(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let root = state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))?;
    let cap = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 50_000);
    Ok(palette_service::index_files(&root, cap))
}
