use std::path::PathBuf;

use tauri::State;

use crate::error::AppResult;
use crate::services::skills_service::{self, Skill};
use crate::state::AppState;

#[tauri::command]
pub fn skill_list(state: State<'_, AppState>) -> AppResult<Vec<Skill>> {
    let project_root = state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.clone());
    skills_service::list(project_root.as_deref())
}

#[tauri::command]
pub fn skill_read(path: String) -> AppResult<String> {
    skills_service::read_md(&PathBuf::from(path))
}
