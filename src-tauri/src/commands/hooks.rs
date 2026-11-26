use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::services::firewall_service::{self, FirewallStatus};
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

/// Firewall mode: are the console's hooks and baseline policy installed in
/// Claude Code's managed settings (and Codex's requirements file), owned by
/// the administrator, and what this version would install? Installing is the user's action; this only
/// reads. With a project, a change since the ledger's last `firewall_state`
/// line is witnessed there (see `witness_firewall`).
#[tauri::command(async)]
pub fn firewall_status(
    project_root: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<FirewallStatus> {
    let status = firewall_service::status();
    if let Some(root) = project_root.as_deref().filter(|r| !r.is_empty()) {
        witness_firewall(&state.testigo, root, &status);
    }
    Ok(status)
}

/// Write `firewall_state` to the project's ledger when the mode is not what
/// that ledger last saw. Best effort: a project with witnessing off, or a
/// ledger that cannot be read, never fails the caller.
pub fn witness_firewall(
    testigo: &crate::services::testigo_service::TestigoService,
    project_root: &str,
    status: &FirewallStatus,
) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Err(e) = testigo.on_firewall_state(
        project_root,
        ts,
        &status.state,
        status.installed_sha256.as_deref(),
        status.expected_sha256.as_deref(),
        &status.problems,
        status.codex.as_ref(),
    ) {
        tracing::debug!("firewall_state not witnessed: {e}");
    }
}
