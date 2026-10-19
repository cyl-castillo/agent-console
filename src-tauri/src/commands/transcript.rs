use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

/// F5: follow the transcript the CLI writes for `session_id` (Claude's
/// `<session>.jsonl` or Codex's rollout) and stream `transcript://delta`
/// while the conversation shows it. Idempotent per session.
#[tauri::command(async)]
pub fn transcript_watch(
    session_id: String,
    engine: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<()> {
    state.transcripts.watch(app, &session_id, &engine)
}

#[tauri::command(async)]
pub fn transcript_unwatch(session_id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.transcripts.unwatch(&session_id);
    Ok(())
}
