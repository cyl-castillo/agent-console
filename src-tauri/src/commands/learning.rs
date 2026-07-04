use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::activity_service::ActivityEvent;
use crate::services::learning_service::{self, CurationResult, ReflectionResult};
use crate::services::{advisor_service, memory_service, skills_service};
use crate::state::AppState;

/// Default activity window for a reflection pass: enough recent events to find
/// real patterns without an unbounded prompt.
const DEFAULT_WINDOW: usize = 400;

fn project_root(state: &State<'_, AppState>) -> AppResult<std::path::PathBuf> {
    state
        .inner
        .lock()
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

/// Curate the existing skill/memory corpus and return improvement suggestions.
/// Read-only: it never mutates the corpus — the UI applies accepted suggestions.
#[tauri::command]
pub async fn learning_curate(state: State<'_, AppState>) -> AppResult<CurationResult> {
    let root = project_root(&state)?;
    // The curator weighs skills by how often they were invoked; reuse the same
    // ledger window the reflection pass reads.
    let events = state
        .activity
        .list(&root.to_string_lossy(), Some(DEFAULT_WINDOW))?;
    // Offload the blocking `claude -p` call so we don't stall the runtime.
    tokio::task::spawn_blocking(move || learning_service::curate(&root, &events))
        .await
        .map_err(|e| AppError::Other(format!("curation task panicked: {e}")))?
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

fn unknown_kind(kind: &str) -> AppError {
    AppError::InvalidArgument(format!("unknown target kind: {kind}"))
}

/// Apply a curation "refactor": overwrite a single entry's content in place.
#[tauri::command]
pub fn learning_apply_refactor(
    state: State<'_, AppState>,
    target_kind: String,
    name: String,
    new_content: String,
) -> AppResult<String> {
    let root = project_root(&state)?;
    let path = match target_kind.as_str() {
        "skill" => skills_service::write(&root, &name, &new_content)?,
        "memory" => memory_service::write(&root, &name, &new_content)?,
        other => return Err(unknown_kind(other)),
    };
    Ok(path.display().to_string())
}

/// Apply a curation "merge": write the consolidated entry, then archive every
/// source it replaced (except the surviving name, which the write overwrote).
#[tauri::command]
pub fn learning_apply_merge(
    state: State<'_, AppState>,
    target_kind: String,
    targets: Vec<String>,
    new_name: String,
    new_content: String,
) -> AppResult<String> {
    let root = project_root(&state)?;
    let path = match target_kind.as_str() {
        "skill" => {
            let p = skills_service::write(&root, &new_name, &new_content)?;
            for t in targets.iter().filter(|t| *t != &new_name) {
                skills_service::archive(&root, t)?;
            }
            p
        }
        "memory" => {
            let p = memory_service::write(&root, &new_name, &new_content)?;
            for t in targets.iter().filter(|t| *t != &new_name) {
                memory_service::archive(&root, t)?;
            }
            p
        }
        other => return Err(unknown_kind(other)),
    };
    Ok(path.display().to_string())
}

/// Apply a curation "archive": retire an obsolete/redundant entry (reversible).
#[tauri::command]
pub fn learning_apply_archive(
    state: State<'_, AppState>,
    target_kind: String,
    name: String,
) -> AppResult<String> {
    let root = project_root(&state)?;
    let path = match target_kind.as_str() {
        "skill" => skills_service::archive(&root, &name)?,
        "memory" => memory_service::archive(&root, &name)?,
        other => return Err(unknown_kind(other)),
    };
    Ok(path.display().to_string())
}
