use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::services::hooks_service::HooksStatus;
use crate::services::sandbox_service::{self, SandboxStatus};
use crate::state::AppState;

#[tauri::command(async)]
pub fn hooks_status(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    Ok(state.hooks.status())
}

#[tauri::command(async)]
pub fn hooks_install(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    state.hooks.install()
}

#[tauri::command(async)]
pub fn hooks_uninstall(state: State<'_, AppState>) -> AppResult<HooksStatus> {
    state.hooks.uninstall()
}

#[tauri::command]
pub fn hooks_start_watcher(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.hooks.start_watcher(app);
    Ok(())
}

#[tauri::command(async)]
pub fn approvals_pending(state: State<'_, AppState>) -> AppResult<Vec<serde_json::Value>> {
    Ok(state.hooks.pending_approvals())
}

#[tauri::command(async)]
pub fn approval_respond(
    id: String,
    decision: String,
    reason: Option<String>,
    policy: Option<String>,
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
            policy.as_deref(),
        );
    }
    Ok(())
}

/// SEGURIDAD-MOVIL F2: is the engine's sandbox on for this project? With
/// it on, the agent's shell cannot write the console's answer files, so a
/// forged approval is impossible rather than detected.
#[tauri::command(async)]
pub fn sandbox_status(project_root: String) -> AppResult<SandboxStatus> {
    Ok(sandbox_service::status(std::path::Path::new(&project_root)))
}

#[tauri::command(async)]
pub fn sandbox_enable(project_root: String) -> AppResult<SandboxStatus> {
    sandbox_service::enable(std::path::Path::new(&project_root))
}
