use tauri::State;

use crate::error::AppResult;
use crate::services::permissions_service::{
    self, Effect, Engine, PermissionsSnapshot, Scope, StoredRule,
};
use crate::state::AppState;

#[tauri::command]
pub fn permissions_snapshot(state: State<'_, AppState>) -> AppResult<PermissionsSnapshot> {
    let project = state.inner.lock().project.clone();
    permissions_service::snapshot(project.as_ref().map(|p| p.root.as_path()))
}

#[tauri::command]
pub fn permissions_add(
    scope: Scope,
    effect: Effect,
    raw: String,
    engine: Option<Engine>,
    state: State<'_, AppState>,
) -> AppResult<StoredRule> {
    let project = state.inner.lock().project.clone();
    permissions_service::add_rule(
        project.as_ref().map(|p| p.root.as_path()),
        scope,
        effect,
        &raw,
        engine.unwrap_or_default(),
    )
}

#[tauri::command]
pub fn permissions_remove(
    scope: Scope,
    effect: Effect,
    raw: String,
    engine: Option<Engine>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let project = state.inner.lock().project.clone();
    permissions_service::remove_rule(
        project.as_ref().map(|p| p.root.as_path()),
        scope,
        effect,
        &raw,
        engine.unwrap_or_default(),
    )
}
