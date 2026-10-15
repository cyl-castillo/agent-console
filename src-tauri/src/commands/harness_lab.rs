use tauri::{AppHandle, Emitter, State};

use crate::error::AppResult;
use crate::services::harness_lab::{self, Experiment, HeldOut, HeldOutTask, LabConfig, LabSummary};
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tauri::command(async)]
pub fn harness_lab_get(project_root: String) -> AppResult<LabSummary> {
    harness_lab::summary(&project_root)
}

#[tauri::command(async)]
pub fn harness_lab_set_config(project_root: String, config: LabConfig) -> AppResult<LabConfig> {
    harness_lab::set_config(&project_root, config)
}

/// The held-out tasks live in the console's data dir, never in the repo, and
/// are never shown to the agent — only the human edits them here.
#[tauri::command(async)]
pub fn harness_lab_set_heldout(
    project_root: String,
    tasks: Vec<HeldOutTask>,
) -> AppResult<HeldOut> {
    harness_lab::set_heldout(&project_root, tasks)
}

/// D12: run one experiment now (blocking: a worktree, the edit, the hidden
/// window twice). Emits `harness://experiment` when it finishes.
#[tauri::command(async)]
pub fn harness_lab_run(
    project_root: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Experiment> {
    let exp = harness_lab::run_experiment(&state.testigo, &project_root, now_ms())?;
    let _ = app.emit("harness://experiment", &exp);
    Ok(exp)
}
