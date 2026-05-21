use std::path::PathBuf;

use crate::error::AppResult;
use crate::services::projects_service::{self, RecentProject};

#[tauri::command]
pub fn projects_recent() -> AppResult<Vec<RecentProject>> {
    Ok(projects_service::load())
}

#[tauri::command]
pub fn projects_last() -> AppResult<Option<RecentProject>> {
    Ok(projects_service::last())
}

#[tauri::command]
pub fn projects_forget(path: String) -> AppResult<()> {
    projects_service::forget(&path)
}

#[tauri::command]
pub fn projects_remember(path: String) -> AppResult<()> {
    projects_service::remember(&PathBuf::from(path))
}
