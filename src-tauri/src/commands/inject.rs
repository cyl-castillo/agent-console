//! Memory injection: per-project toggle + the recent-injections feed the GUI
//! shows so nothing is ever fed to the agent silently.

use crate::error::AppResult;
use crate::services::inject_service::{self, InjectionRecord};

#[tauri::command]
pub fn memory_injection_enabled(project_root: String) -> bool {
    inject_service::is_enabled(&project_root)
}

#[tauri::command]
pub fn memory_injection_set_enabled(project_root: String, enabled: bool) -> AppResult<()> {
    inject_service::set_enabled(&project_root, enabled)
}

#[tauri::command]
pub fn memory_injection_recent() -> Vec<InjectionRecord> {
    inject_service::recent()
}
