use tauri::State;

use crate::error::AppResult;
use crate::services::vault_service::{self, Scope, VaultEntryView};
use crate::state::AppState;

fn project_root(state: &AppState) -> Option<std::path::PathBuf> {
    state.inner.lock().project.as_ref().map(|p| p.root.clone())
}

#[tauri::command]
pub fn vault_list(state: State<'_, AppState>) -> AppResult<Vec<VaultEntryView>> {
    vault_service::list(project_root(&state).as_deref())
}

#[tauri::command]
pub fn vault_upsert(
    state: State<'_, AppState>,
    scope: Scope,
    key: String,
    description: String,
    secret: bool,
    value: Option<String>,
) -> AppResult<VaultEntryView> {
    vault_service::upsert(
        project_root(&state).as_deref(),
        scope,
        key,
        description,
        secret,
        value,
    )
}

#[tauri::command]
pub fn vault_delete(state: State<'_, AppState>, scope: Scope, key: String) -> AppResult<()> {
    vault_service::delete(project_root(&state).as_deref(), scope, &key)
}

#[tauri::command]
pub fn vault_get_value(state: State<'_, AppState>, scope: Scope, key: String) -> AppResult<String> {
    vault_service::get_value(project_root(&state).as_deref(), scope, &key)
}
