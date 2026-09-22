use tauri::State;

use crate::error::AppResult;
use crate::services::diagnostics::{self, DiagnosticsBundle};
use crate::state::AppState;

/// Collect the bundle from live state. Runs off the main thread: preflight
/// spawns the CLIs and the store sizes walk two directories.
pub fn collect(state: &AppState) -> DiagnosticsBundle {
    let (project_root, project_name) = {
        let s = state.inner.lock();
        (
            s.project.as_ref().map(|p| p.root.clone()),
            s.project.as_ref().map(|p| p.name.clone()),
        )
    };
    let project_branch = crate::services::feedback_service::context(
        project_root.as_deref(),
        project_name.as_deref(),
    )
    .branch;
    let data_dir = dirs::data_local_dir().map(|d| d.join("agent-console"));
    let cache_dir = dirs::cache_dir().map(|d| d.join("agent-console"));
    let log_file = diagnostics::log_dir().and_then(|d| diagnostics::current_log_file(&d));
    let log_tail = log_file
        .as_deref()
        .map(|f| diagnostics::tail_lines(f, diagnostics::LOG_TAIL_LINES))
        .unwrap_or_default();
    DiagnosticsBundle {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        commit: env!("AC_BUILD_COMMIT").to_string(),
        build_time_secs: env!("AC_BUILD_TIME").parse().unwrap_or(0),
        debug: cfg!(debug_assertions),
        snap: std::env::var_os("SNAP").is_some(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        project_root: project_root.map(|p| p.to_string_lossy().to_string()),
        project_branch,
        hooks: serde_json::to_value(state.hooks.status()).unwrap_or_default(),
        preflight: serde_json::to_value(super::preflight::preflight_check()).unwrap_or_default(),
        inject_port_file: data_dir
            .as_ref()
            .is_some_and(|d| d.join("inject-port.json").is_file()),
        data_entries: data_dir
            .as_deref()
            .map(diagnostics::dir_report)
            .unwrap_or_default(),
        data_dir: data_dir.map(|d| d.to_string_lossy().to_string()),
        cache_entries: cache_dir
            .as_deref()
            .map(diagnostics::dir_report)
            .unwrap_or_default(),
        cache_dir: cache_dir.map(|d| d.to_string_lossy().to_string()),
        log_file: log_file.map(|f| f.to_string_lossy().to_string()),
        log_tail,
    }
}

/// Markdown diagnostics for "Copy diagnostics" / "Report a problem".
#[tauri::command(async)]
pub fn diagnostics_bundle(state: State<'_, AppState>) -> AppResult<String> {
    Ok(diagnostics::render(&collect(&state)))
}

/// Path of the current log file (to reveal it in the file manager), or an
/// error when logging never got a writable directory.
#[tauri::command(async)]
pub fn diagnostics_log_file() -> AppResult<String> {
    diagnostics::log_dir()
        .and_then(|d| diagnostics::current_log_file(&d))
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| crate::error::AppError::NotFound("no log file yet".into()))
}
