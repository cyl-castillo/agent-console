use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::services::assistant_log::{self, LogSummary, NewEntry, ProposalSet};
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

/// What the panel asks for one turn.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantTurn {
    pub message: String,
    pub engine: Engine,
    #[serde(default)]
    pub model: Option<String>,
    /// The `sessionId` of the previous reply; without it the turn starts a
    /// new conversation.
    #[serde(default)]
    pub resume: Option<String>,
    /// The human's opt-in to the learning log (`assistant_log`); without it
    /// nothing about the turn's content is stored anywhere.
    #[serde(default)]
    pub remember: bool,
    /// The question came by voice.
    #[serde(default)]
    pub spoken: bool,
}

/// One turn of the console's assistant on the engine the human chose.
#[tauri::command]
pub async fn assistant_ask(
    app: AppHandle,
    state: State<'_, AppState>,
    turn: AssistantTurn,
) -> AppResult<AssistantReply> {
    let AssistantTurn {
        message,
        engine,
        model,
        resume,
        remember,
        spoken,
    } = turn;
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
    let question = message.clone();
    let (mut result, tools) = tokio::task::spawn_blocking(move || {
        // The console tools it called, for the learning log.
        let tools = std::cell::RefCell::new(Vec::<String>::new());
        // One event per change of activity, not per streamed delta.
        let last = std::cell::RefCell::new((String::new(), String::new()));
        let sink = |kind: &str, label: &str, _text: &str| {
            let mut last = last.borrow_mut();
            if last.0 == kind && last.1 == label {
                return;
            }
            *last = (kind.to_string(), label.to_string());
            if kind == "tool" {
                tools
                    .borrow_mut()
                    .push(label.trim_start_matches("mcp__agent_console__").to_string());
            }
            let _ = emitter.emit(
                "assistant://activity",
                AssistantActivity {
                    kind: kind.into(),
                    label: label.into(),
                },
            );
        };
        let result = assistant_service::ask(&AskSpec {
            project_root: &task_root,
            bridge: bridge.as_deref(),
            engine,
            model: model.as_deref(),
            resume: resume.as_deref(),
            message: &message,
            on_activity: Some(&sink),
        });
        (result, tools.into_inner())
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

    // The learning log, only when the human switched it on. Failures are
    // kept too: a turn that broke is the clearest sign of a gap. A log that
    // cannot be written never fails the answer.
    if remember {
        let error = result.as_ref().err().map(|e| e.to_string());
        let logged = assistant_log::record(
            &root.to_string_lossy(),
            &NewEntry {
                ts: now_ms(),
                question: &question,
                answer: result.as_ref().map(|r| r.text.as_str()).unwrap_or(""),
                tools: &tools,
                error: error.as_deref(),
                engine,
                tokens: result.as_ref().map(|r| r.tokens).unwrap_or(0),
                spoken,
            },
        );
        match (logged, result.as_mut()) {
            (Ok(id), Ok(reply)) => reply.log_id = Some(id),
            (Err(e), _) => tracing::warn!("assistant log: {e}"),
            _ => {}
        }
    }
    result
}

fn project_root(state: &State<'_, AppState>) -> AppResult<std::path::PathBuf> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::Other("Open a project first".into()))
}

/// Counts, the file's location and the last proposals of this project's
/// learning log.
#[tauri::command]
pub async fn assistant_log_summary(state: State<'_, AppState>) -> AppResult<LogSummary> {
    let root = project_root(&state)?;
    assistant_log::summary(&root.to_string_lossy())
}

/// The human's rating of one logged turn: "up", "down", or none to clear it.
#[tauri::command]
pub async fn assistant_log_rate(
    state: State<'_, AppState>,
    id: String,
    rating: Option<String>,
) -> AppResult<()> {
    let root = project_root(&state)?;
    assistant_log::rate(&root.to_string_lossy(), &id, rating.as_deref())
}

/// Delete this project's learning log and its proposals.
#[tauri::command]
pub async fn assistant_log_clear(state: State<'_, AppState>) -> AppResult<()> {
    let root = project_root(&state)?;
    assistant_log::clear(&root.to_string_lossy())
}

/// One turn of the human's engine over the log: where does the console fall
/// short. Witnessed like any other turn the assistant runs.
#[tauri::command]
pub async fn assistant_propose(
    state: State<'_, AppState>,
    engine: Engine,
    model: Option<String>,
) -> AppResult<ProposalSet> {
    let root = project_root(&state)?;
    let model = model.filter(|m| !m.trim().is_empty());
    let task_root = root.clone();
    let result = tokio::task::spawn_blocking(move || {
        assistant_log::propose(&task_root, engine, model.as_deref(), now_ms())
    })
    .await
    .map_err(|e| AppError::Other(format!("assistant propose task panicked: {e}")))?;
    let _ = state.testigo.on_agent_action(
        &root.to_string_lossy(),
        now_ms(),
        CALLER,
        "assistant_propose",
        "",
        result.is_ok(),
        result.as_ref().err().map(|e| e.to_string()).as_deref(),
        None,
    );
    result
}
