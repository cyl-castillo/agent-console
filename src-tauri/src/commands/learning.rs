use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::activity_service::ActivityEvent;
use crate::services::learning_service::{self, ReflectionResult};
use crate::services::{advisor_service, memory_service};
use crate::state::AppState;

/// Default activity window for a reflection pass: enough recent events to find
/// real patterns without an unbounded prompt.
const DEFAULT_WINDOW: usize = 400;

fn project_root(state: &State<'_, AppState>) -> AppResult<std::path::PathBuf> {
    state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::Other("no project open".into()))
}

/// Reflect over recent recorded activity and return improvement suggestions.
#[tauri::command]
pub async fn learning_reflect(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> AppResult<ReflectionResult> {
    let root = project_root(&state)?;
    let events = state.activity.list(
        &root.to_string_lossy(),
        Some(limit.unwrap_or(DEFAULT_WINDOW)),
    )?;
    // Offload the blocking `claude -p` call so we don't stall the runtime.
    tokio::task::spawn_blocking(move || learning_service::reflect(&root, &events))
        .await
        .map_err(|e| AppError::Other(format!("reflection task panicked: {e}")))?
}

/// Read back the raw activity ledger (most recent first), for inspection.
#[tauri::command]
pub fn activity_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> AppResult<Vec<ActivityEvent>> {
    let root = project_root(&state)?;
    let mut events = state.activity.list(&root.to_string_lossy(), limit)?;
    events.reverse(); // newest first for display
    Ok(events)
}

/// Materialize an accepted "skill" suggestion. Learned skills are project-scoped
/// (they came from this project's activity), reusing the Advisor's writer.
#[tauri::command]
pub fn learning_create_skill(
    state: State<'_, AppState>,
    name: String,
    skill_md_content: String,
) -> AppResult<String> {
    let root = project_root(&state)?;
    let path = advisor_service::create_skill(&root, "project", &name, &skill_md_content)?;
    Ok(path.display().to_string())
}

/// Materialize an accepted "memory" suggestion into the project's memory dir.
#[tauri::command]
pub fn learning_save_memory(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> AppResult<String> {
    let root = project_root(&state)?;
    let path = memory_service::write(&root, &name, &content)?;
    Ok(path.display().to_string())
}
