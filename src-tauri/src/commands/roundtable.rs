use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::services::roundtable_service::{
    self, ConnectorView, PersistedRoom, RoomSummary, RoundtableConfig, ShareResult, SyncResult,
};
use crate::state::AppState;

/// The open project's root, or an error if none is open. Persisted rooms are
/// keyed by it, exactly as `roundtable_start` keys the live run.
fn project_root(state: &State<'_, AppState>) -> AppResult<String> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.display().to_string())
        .ok_or_else(|| AppError::Other("no project open".into()))
}

#[tauri::command(async)]
pub fn roundtable_start(
    app: AppHandle,
    state: State<'_, AppState>,
    config: RoundtableConfig,
) -> AppResult<String> {
    let repo = state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::Other("no project open".into()))?;
    state.roundtable.start(app, repo, config)
}

#[tauri::command]
pub fn roundtable_pause(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.pause(&id)
}

#[tauri::command]
pub fn roundtable_resume(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.resume(&id)
}

#[tauri::command]
pub fn roundtable_inject(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    message: String,
) -> AppResult<()> {
    state.roundtable.inject(&app, &id, message)
}

#[tauri::command]
pub fn roundtable_continue(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    extra: u32,
) -> AppResult<()> {
    state.roundtable.continue_run(&app, &id, extra)
}

#[tauri::command]
pub fn roundtable_stop(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.stop(&id)
}

#[tauri::command(async)]
pub fn roundtable_discard(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.roundtable.discard(&id)
}

/// Share a working room with collaborators: push its `room/<id>` branch to the
/// shared remote and return an MR/PR link the human can hand to a colleague.
#[tauri::command(async)]
pub fn roundtable_share(state: State<'_, AppState>, id: String) -> AppResult<ShareResult> {
    state.roundtable.share(&id)
}

/// Sync a colleague's commits into a live working room: fetch its `room/<id>`
/// branch from the remote and merge them into the worktree so the next turn
/// builds on top. The inbound mirror of `roundtable_share`.
#[tauri::command(async)]
pub fn roundtable_sync(state: State<'_, AppState>, id: String) -> AppResult<SyncResult> {
    state.roundtable.sync(&id)
}

/// Persisted rooms for the open project (lightweight, for the sidebar list).
#[tauri::command(async)]
pub fn roundtable_list_rooms(state: State<'_, AppState>) -> AppResult<Vec<RoomSummary>> {
    let root = project_root(&state)?;
    state.roundtable.rooms().summaries(&root)
}

/// Full saved state of one room, for read-only re-hydration.
#[tauri::command(async)]
pub fn roundtable_get_room(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<PersistedRoom>> {
    let root = project_root(&state)?;
    state.roundtable.rooms().get(&root, &id)
}

/// Drop a saved room from this project's history. Idempotent.
#[tauri::command(async)]
pub fn roundtable_delete_room(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let root = project_root(&state)?;
    state.roundtable.rooms().delete_room(&root, &id)?;
    // The room's connector records (team, tasks, questions, reviews) go with it.
    state.connector.forget_job(&root, &id)
}

/// Rebuild a live run from a saved room so it can be continued (Fase B). Returns
/// the (unchanged) room id, now registered as a live run in the "awaiting" state.
#[tauri::command(async)]
pub fn roundtable_resume_room(state: State<'_, AppState>, id: String) -> AppResult<String> {
    let root = project_root(&state)?;
    let room: PersistedRoom = state
        .roundtable
        .rooms()
        .get(&root, &id)?
        .ok_or_else(|| AppError::NotFound(format!("saved room {id}")))?;
    state.roundtable.restore(PathBuf::from(&root), room)
}

/// The connector's view of a room: team, delegated tasks, questions to the
/// human and recorded reviews. Works for live and saved rooms alike (the
/// connector keys by project root + room id, not by live run).
#[tauri::command(async)]
pub fn roundtable_connector_state(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<ConnectorView> {
    let root = project_root(&state)?;
    roundtable_service::connector_view(&state.connector, &root, &id)
}

/// Answer an agent's `ask_user` question: by option id (`choice_id`) or with
/// free text (`body`). The room resumes with the asker's turn.
#[tauri::command(async)]
pub fn roundtable_answer_question(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    question_id: String,
    body: String,
    choice_id: Option<String>,
) -> AppResult<()> {
    state
        .roundtable
        .answer_question(&app, &id, &question_id, &body, choice_id.as_deref())
}

/// Resolve a task an agent left waiting for approval (`create_task`).
/// Discarding closes it; approving starts it as a new job room with the same
/// team (see `spawn_followup`) and returns the record carrying that room's id.
#[tauri::command(async)]
pub fn roundtable_resolve_pending(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    pending_id: String,
    approve: bool,
) -> AppResult<crate::services::connector_service::PendingJob> {
    let root = project_root(&state)?;
    // Guard: the pending task must belong to this room.
    let belongs = state
        .connector
        .pending_jobs(&root, &id)?
        .iter()
        .any(|j| j.id == pending_id);
    if !belongs {
        return Err(AppError::NotFound(format!("pending task {pending_id}")));
    }
    if approve {
        return state.roundtable.spawn_followup(&app, &id, &pending_id);
    }
    state.connector.resolve_pending(&root, &pending_id, None)
}
