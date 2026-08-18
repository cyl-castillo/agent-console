use tauri::{AppHandle, State};

use std::path::PathBuf;

use crate::error::AppResult;
use crate::services::hooks_service::HooksStatus;
use crate::services::workspace_trust::{self, TrustState, WorkspaceTrust};
use crate::state::AppState;

#[tauri::command]
pub fn hooks_status(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    Ok(state.hooks.status())
}

/// Whether each CLI trusts `dir` (defaults to the active project root).
/// Installed hooks are inert in an untrusted directory, and the CLIs say so
/// only inside their own trust prompt — this is what lets the console warn
/// instead of showing an "active" integration that never fires.
#[tauri::command]
pub fn hooks_trust_status(
    dir: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<WorkspaceTrust> {
    let dir = dir
        .map(PathBuf::from)
        .or_else(|| state.inner.lock().project.as_ref().map(|p| p.root.clone()));
    Ok(match dir {
        Some(d) => workspace_trust::trust_for(&d),
        // No project open: nothing to judge, and silence beats a false alarm.
        None => WorkspaceTrust {
            dir: PathBuf::new(),
            claude: TrustState::Unknown,
            codex: TrustState::Unknown,
        },
    })
}

#[tauri::command]
pub fn hooks_install(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    state.hooks.install()
}

#[tauri::command]
pub fn hooks_uninstall(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    state.hooks.uninstall()
}

#[tauri::command]
pub fn hooks_start_watcher(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.hooks.start_watcher(app);
    Ok(())
}

#[tauri::command]
pub fn approvals_pending(state: State<'_, AppState>) -> AppResult<Vec<serde_json::Value>> {
    Ok(state.hooks.pending_approvals())
}

#[tauri::command]
pub fn approval_respond(
    id: String,
    decision: String,
    reason: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.hooks.respond(&id, &decision, reason.as_deref())?;
    // Testigo: the human half of the approval audit trail. The res file the
    // hook polls is deleted after pickup; this ledger line is what remains.
    // Best-effort: a ledger failure must never turn a granted approval into
    // an error toast.
    let project = state.inner.lock().project.clone();
    if let Some(p) = project {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let root = p.root.to_string_lossy();
        let _ = state.testigo.on_approval_decision(
            root.as_ref(),
            ts,
            &id,
            &decision,
            reason.as_deref(),
        );
    }
    Ok(())
}
