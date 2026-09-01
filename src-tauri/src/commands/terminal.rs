use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn term_spawn(
    cwd: String,
    term_key: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let session_dir = state.hooks.session_dir().to_string_lossy().to_string();
    let mut extra = vec![
        ("AGENT_CONSOLE_SESSION_DIR".to_string(), session_dir),
        ("AGENT_CONSOLE_BRIDGE".to_string(), "1".to_string()),
    ];
    // Tag this PTY with the frontend terminal-session id so the UserPromptSubmit
    // hook can attribute the claude session id to THIS terminal deterministically
    // (see userprompt-hook.cjs / skillsStore._onPrompt). Without it, the UI would
    // fall back to "whatever session is active", which misbinds when more than
    // one claude runs at a time and breaks `--resume`.
    if let Some(key) = term_key {
        if !key.is_empty() {
            extra.push((
                crate::services::terminal_runner::TERM_ID_ENV.to_string(),
                key,
            ));
        }
    }
    // Inject Vault entries (project overrides global) so the agent can use
    // `$KEY` in shell commands without ever seeing the value in its context.
    let project_root = state.inner.lock().project.as_ref().map(|p| p.root.clone());
    if let Ok(vault_env) = crate::services::vault_service::env_for_spawn(project_root.as_deref()) {
        for (k, v) in vault_env {
            extra.push((k, v));
        }
    }
    state
        .terminals
        .spawn_with_env(app, &PathBuf::from(cwd), &extra)
}

/// Which live Claude session each terminal is actually running, proven by
/// process ancestry (see `agent_sessions`). The frontend polls this so a
/// terminal learns its resume handle even when no hook ever fired for it.
/// Terminals with no match are simply absent from the result.
#[tauri::command]
pub fn term_agent_sessions(
    state: State<'_, AppState>,
) -> Vec<crate::services::agent_sessions::TermBinding> {
    crate::services::agent_sessions::reconcile(&state.terminals.live_shells())
}

#[tauri::command]
pub fn term_write(id: String, data: String, state: State<'_, AppState>) -> AppResult<()> {
    state.terminals.write(&id, data.as_bytes())
}

#[tauri::command]
pub fn term_resize(id: String, cols: u16, rows: u16, state: State<'_, AppState>) -> AppResult<()> {
    state.terminals.resize(&id, cols, rows)
}

#[tauri::command]
pub fn term_kill(id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.terminals.kill(&id)
}

/// Save image bytes pasted into a terminal to a temp file and return its
/// absolute path. The frontend then types that path into the agent composer
/// (same flow as dragging an image file onto the terminal). Raw-body command:
/// the bytes arrive as `InvokeBody::Raw`, the extension via the
/// `x-image-ext` header (allowlisted, defaults to png).
#[tauri::command]
pub fn term_save_paste_image(request: tauri::ipc::Request<'_>) -> AppResult<String> {
    use std::sync::atomic::{AtomicU64, Ordering};

    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(crate::error::AppError::InvalidArgument(
            "expected raw image bytes".into(),
        ));
    };
    if bytes.is_empty() {
        return Err(crate::error::AppError::InvalidArgument(
            "empty image payload".into(),
        ));
    }
    let ext = request
        .headers()
        .get("x-image-ext")
        .and_then(|v| v.to_str().ok())
        .filter(|e| matches!(*e, "png" | "jpg" | "gif" | "webp" | "bmp"))
        .unwrap_or("png");

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("agent-console-paste-{millis}-{n}.{ext}"));
    std::fs::write(&path, bytes)?;
    Ok(path.to_string_lossy().to_string())
}
