//! F9: fan-out — N agents on the same prompt in twin worktrees, compared,
//! one applied by the human.

use std::sync::Arc;

use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::services::fanout_service::{FanOut, FanOutConfig, FanOutService, Notify};
use crate::services::worktree_service::MergeOutcome;
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn repo_of(state: &AppState) -> AppResult<std::path::PathBuf> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::Other("no project open".into()))
}

#[tauri::command(async)]
pub fn fanout_start(
    config: FanOutConfig,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let repo = repo_of(&state)?;
    let notify_app = app.clone();
    let root = repo.to_string_lossy().to_string();
    let notify: Notify = Arc::new(move |id: &str| {
        // Every change reaches the UI; the finished comparison reaches the ledger.
        let _ = notify_app.emit("fanout://changed", &json!({ "id": id, "project": root }));
        let st = tauri::Manager::state::<AppState>(&notify_app);
        if let Some(fo) = st.fanout.get(id) {
            if fo.all_done() && !fo.closed {
                let _ = st.testigo.on_fanout(
                    &fo.project,
                    now_ms(),
                    "fanout_result",
                    "console",
                    FanOutService::result_payload(&fo),
                );
            }
        }
    });
    state.fanout.start(
        &repo,
        config,
        crate::services::engine_runner::runner_for,
        notify,
    )
}

#[tauri::command(async)]
pub fn fanout_list(state: State<'_, AppState>) -> AppResult<Vec<FanOut>> {
    let repo = repo_of(&state)?;
    Ok(state.fanout.list(&repo.to_string_lossy()))
}

#[tauri::command(async)]
pub fn fanout_get(id: String, state: State<'_, AppState>) -> AppResult<Option<FanOut>> {
    Ok(state.fanout.get(&id))
}

/// The human's pick: merge that candidate onto the base; witnessed as
/// `fanout_decision` with the reason, before the merge runs.
#[tauri::command(async)]
pub fn fanout_apply(
    id: String,
    participant_id: String,
    reason: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<MergeOutcome> {
    let repo = repo_of(&state)?;
    let fo = state
        .fanout
        .get(&id)
        .ok_or_else(|| AppError::NotFound(format!("fan-out {id}")))?;
    let _ = state.testigo.on_fanout(
        &fo.project,
        now_ms(),
        "fanout_decision",
        "human",
        json!({ "id": id, "decision": "apply", "participant": participant_id, "reason": reason,
                "candidates": fo.candidates.len() }),
    );
    let out = state.fanout.apply(&repo, &id, &participant_id)?;
    let _ = app.emit("fanout://changed", &json!({ "id": id }));
    Ok(out)
}

#[tauri::command(async)]
pub fn fanout_discard(id: String, app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let repo = repo_of(&state)?;
    if let Some(fo) = state.fanout.get(&id) {
        let _ = state.testigo.on_fanout(
            &fo.project,
            now_ms(),
            "fanout_decision",
            "human",
            json!({ "id": id, "decision": "discard", "candidates": fo.candidates.len() }),
        );
    }
    state.fanout.discard(&repo, &id)?;
    let _ = app.emit("fanout://changed", &json!({ "id": id }));
    Ok(())
}
