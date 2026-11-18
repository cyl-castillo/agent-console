use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::services::assistant_service::{self, AskSpec, AssistantReply, CALLER};
use crate::services::engine_runner::Engine;
use crate::state::AppState;

/// Live activity of the turn in flight (`assistant://activity`): thinking,
/// writing, or which tool it is calling. Never the text itself — the reply
/// comes back whole, and a delta per token would flood the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssistantActivity {
    kind: String,
    label: String,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One turn of the console's assistant on the engine the human chose.
/// `resume` is the `sessionId` of the previous reply; without it the turn
/// starts a new conversation.
#[tauri::command]
pub async fn assistant_ask(
    app: AppHandle,
    state: State<'_, AppState>,
    message: String,
    engine: Engine,
    model: Option<String>,
    resume: Option<String>,
) -> AppResult<AssistantReply> {
    let root = state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::Other("Open a project first".into()))?;
    let bridge = state.hooks.bridge_binary().map(|p| p.to_path_buf());
    // Refuse bad input before spending a turn or writing to the ledger.
    let message = assistant_service::clean_message(&message)?;
    let model = model.filter(|m| !m.trim().is_empty());
    let resume = resume.filter(|r| !r.trim().is_empty());

    let task_root = root.clone();
    let emitter = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        // One event per change of activity, not per streamed delta.
        let last = std::cell::RefCell::new((String::new(), String::new()));
        let sink = |kind: &str, label: &str, _text: &str| {
            let mut last = last.borrow_mut();
            if last.0 == kind && last.1 == label {
                return;
            }
            *last = (kind.to_string(), label.to_string());
            let _ = emitter.emit(
                "assistant://activity",
                AssistantActivity {
                    kind: kind.into(),
                    label: label.into(),
                },
            );
        };
        assistant_service::ask(&AskSpec {
            project_root: &task_root,
            bridge: bridge.as_deref(),
            engine,
            model: model.as_deref(),
            resume: resume.as_deref(),
            message: &message,
            on_activity: Some(&sink),
        })
    })
    .await
    .map_err(|e| AppError::Other(format!("assistant task panicked: {e}")))?;

    // The turn itself is witnessed under the assistant's identity — that it
    // ran and how it ended, never the human's words.
    let _ = state.testigo.on_agent_action(
        &root.to_string_lossy(),
        now_ms(),
        CALLER,
        "assistant_turn",
        "",
        result.is_ok(),
        result.as_ref().err().map(|e| e.to_string()).as_deref(),
        None,
    );
    result
}
