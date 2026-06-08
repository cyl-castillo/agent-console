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
            extra.push(("AGENT_CONSOLE_TERM_ID".to_string(), key));
        }
    }
    // Inject Vault entries (project overrides global) so the agent can use
    // `$KEY` in shell commands without ever seeing the value in its context.
    let project_root = state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.clone());
    if let Ok(vault_env) = crate::services::vault_service::env_for_spawn(project_root.as_deref()) {
        for (k, v) in vault_env {
            extra.push((k, v));
        }
    }
    state
        .terminals
        .spawn_with_env(app, &PathBuf::from(cwd), &extra)
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
