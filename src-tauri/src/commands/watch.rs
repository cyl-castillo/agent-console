//! Goal watch (experimental): thin commands over `watch_service`. Every one
//! is scoped to the open project; none writes to the ledger.

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::testigo_service::ProofEvent;
use crate::services::watch_service::{now_ms, Exchange, Goal, SourceStatus};
use crate::state::AppState;

fn project_root(state: &State<'_, AppState>) -> AppResult<String> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string())
        .ok_or_else(|| AppError::Other("no project open".into()))
}

/// Goals of the open project, newest first.
#[tauri::command(async)]
pub fn watch_list(state: State<'_, AppState>) -> AppResult<Vec<Goal>> {
    let root = project_root(&state)?;
    state.watch.list(&root)
}

/// Begin following a goal. `demo` follows the scripted timeline instead of
/// the project's ledger.
#[tauri::command(async)]
pub fn watch_start(state: State<'_, AppState>, title: String, demo: bool) -> AppResult<Goal> {
    let root = project_root(&state)?;
    state
        .watch
        .start(&state.testigo, &root, &title, demo, now_ms())
}

/// active | paused | stopped (stopped is final).
#[tauri::command(async)]
pub fn watch_set_status(state: State<'_, AppState>, id: String, status: String) -> AppResult<Goal> {
    let root = project_root(&state)?;
    state.watch.set_status(&root, &id, &status)
}

#[tauri::command(async)]
pub fn watch_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let root = project_root(&state)?;
    state.watch.delete(&root, &id)
}

/// new | seen | dismissed
#[tauri::command(async)]
pub fn watch_set_alert_state(
    state: State<'_, AppState>,
    goal_id: String,
    alert_id: String,
    alert_state: String,
) -> AppResult<Goal> {
    let root = project_root(&state)?;
    state
        .watch
        .set_alert_state(&root, &goal_id, &alert_id, &alert_state)
}

/// The ledger lines behind an alert (its whole turn when it has one).
#[tauri::command(async)]
pub fn watch_evidence(
    state: State<'_, AppState>,
    goal_id: String,
    alert_id: String,
) -> AppResult<Vec<ProofEvent>> {
    let root = project_root(&state)?;
    state
        .watch
        .evidence(&state.testigo, &root, &goal_id, &alert_id, now_ms())
}

/// Answer a question from the goal's ledger lines. `focus` is the alert the
/// human has open: what "that run" refers to.
#[tauri::command(async)]
pub fn watch_ask(
    state: State<'_, AppState>,
    goal_id: String,
    question: String,
    focus: Option<String>,
) -> AppResult<Exchange> {
    let root = project_root(&state)?;
    state.watch.ask(
        &state.testigo,
        &root,
        &goal_id,
        &question,
        focus.as_deref(),
        now_ms(),
    )
}

/// Whether the project's ledger is being written, and where its head is.
#[tauri::command(async)]
pub fn watch_source(state: State<'_, AppState>) -> AppResult<SourceStatus> {
    let root = project_root(&state)?;
    Ok(state.watch.source_status(&state.testigo, &root))
}
