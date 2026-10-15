use serde_json::json;
use tauri::State;

use crate::error::AppResult;
use crate::services::sponsor_service::Sponsor;
use crate::state::AppState;

#[tauri::command(async)]
pub fn sponsor_get(project_root: String, state: State<'_, AppState>) -> AppResult<Sponsor> {
    Ok(state.sponsor.get(&project_root))
}

/// D11: set the product's sponsor policy; the change is witnessed as
/// `sponsor_set` so the ledger states what the console may decide alone.
#[tauri::command(async)]
pub fn sponsor_set(
    project_root: String,
    sponsor: Sponsor,
    state: State<'_, AppState>,
) -> AppResult<Sponsor> {
    let saved = state.sponsor.set(&project_root, sponsor)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let _ = state
        .testigo
        .on_sponsor_set(&project_root, ts, json!({ "sponsor": saved }));
    Ok(saved)
}
