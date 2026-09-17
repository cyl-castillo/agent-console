use std::path::PathBuf;

use tauri::State;

use crate::error::AppResult;
use crate::services::path_guard;
use crate::services::skills_service::{self, Skill};
use crate::state::AppState;

#[tauri::command(async)]
pub fn skill_list(state: State<'_, AppState>) -> AppResult<Vec<Skill>> {
    let project_root = state.inner.lock().project.as_ref().map(|p| p.root.clone());
    skills_service::list(project_root.as_deref())
}

/// Skills live under the project's `.claude/` or the user's CLI config dirs
/// (`~/.claude`, `~/.codex`) — the same roots `skill_list` scans. A path from
/// the webview outside those is refused, whatever the list said.
#[tauri::command(async)]
pub fn skill_read(path: String, state: State<'_, AppState>) -> AppResult<String> {
    let mut roots = path_guard::cli_config_roots();
    if let Some(root) = state.inner.lock().project.as_ref().map(|p| p.root.clone()) {
        roots.push(root.join(".claude"));
    }
    let path = path_guard::confine(&PathBuf::from(path), &roots)?;
    skills_service::read_md(&path)
}
