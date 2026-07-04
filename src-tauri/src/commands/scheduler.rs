use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::services::scheduler_service::{Job, RunRecord};
use crate::state::AppState;

/// Default cap on returned run-history records for the results feed.
const HISTORY_WINDOW: usize = 100;

fn project_root(state: &State<'_, AppState>) -> AppResult<String> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string())
        .ok_or_else(|| AppError::Other("no project open".into()))
}

/// All scheduled jobs for the open project.
#[tauri::command]
pub fn scheduler_list(state: State<'_, AppState>) -> AppResult<Vec<Job>> {
    let root = project_root(&state)?;
    state.scheduler.list(&root)
}

/// Create (or replace by id) a job.
#[tauri::command]
pub fn scheduler_create(state: State<'_, AppState>, job: Job) -> AppResult<Job> {
    let root = project_root(&state)?;
    state.scheduler.create(&root, job)
}

/// Update a job in place (recomputes its next firing).
#[tauri::command]
pub fn scheduler_update(state: State<'_, AppState>, job: Job) -> AppResult<Job> {
    let root = project_root(&state)?;
    state.scheduler.update(&root, job)
}

#[tauri::command]
pub fn scheduler_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let root = project_root(&state)?;
    state.scheduler.delete(&root, &id)
}

/// Pause or resume a job (`enabled=false` pauses; `true` resumes + reschedules).
#[tauri::command]
pub fn scheduler_set_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> AppResult<Job> {
    let root = project_root(&state)?;
    state.scheduler.set_enabled(&root, &id, enabled)
}

/// Recent run records for the open project, newest first.
#[tauri::command]
pub fn scheduler_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> AppResult<Vec<RunRecord>> {
    let root = project_root(&state)?;
    state
        .scheduler
        .history(&root, Some(limit.unwrap_or(HISTORY_WINDOW)))
}

/// Whether the global scheduler kill-switch is engaged.
#[tauri::command]
pub fn scheduler_is_paused(state: State<'_, AppState>) -> AppResult<bool> {
    Ok(state.scheduler.is_paused())
}

/// Engage/release the global kill-switch. When paused, the tick loop and event
/// firing run nothing; an explicit "run now" still works.
#[tauri::command]
pub fn scheduler_set_paused(
    app: AppHandle,
    state: State<'_, AppState>,
    paused: bool,
) -> AppResult<()> {
    state.scheduler.set_paused(&app, paused)
}

/// Fire any event-triggered jobs registered for `name` (e.g. "commit",
/// "corpus_grew", "prompt"). Returns immediately; runs are offloaded.
#[tauri::command]
pub fn scheduler_fire_event(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> AppResult<()> {
    let root = project_root(&state)?;
    state.scheduler.fire_event(&app, &root, &name);
    Ok(())
}

/// Run a job immediately (off the runtime thread, since it spawns `claude`).
#[tauri::command]
pub async fn scheduler_run_now(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<RunRecord> {
    let root = project_root(&state)?;
    // The service is held in managed state; clone the handle and re-fetch it
    // inside the blocking task so we don't hold the State across the await.
    tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        st.scheduler.run_now(&app, &root, &id)
    })
    .await
    .map_err(|e| AppError::Other(format!("run-now task panicked: {e}")))?
}
