use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::error::AppResult;
use crate::services::trust_service::TrustStatus;
use crate::state::AppState;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// F4: what the folder would inject into the agents, and where the human's
/// last word stands against it.
#[tauri::command(async)]
pub fn trust_status(project_root: String, state: State<'_, AppState>) -> AppResult<TrustStatus> {
    state.trust.status(&project_root)
}

/// F4: the human's word on the folder as it is now — persisted, witnessed
/// as `trust_decision`, announced as `console://trust` so the Inbox card
/// and the waiting terminals move.
#[tauri::command(async)]
pub fn trust_decide(
    project_root: String,
    decision: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<TrustStatus> {
    let ts = now_ms();
    let status = state.trust.decide(&project_root, &decision, ts)?;
    let _ = state.testigo.on_trust_decision(
        &project_root,
        ts as i64,
        &decision,
        &status.surface.digest,
        json!(status.surface.counts),
    );
    let _ = app.emit(
        "console://trust",
        &json!({ "project": project_root, "state": status.state }),
    );
    Ok(status)
}
