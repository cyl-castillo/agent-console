use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::mcp_service::{self, McpAddInput, McpServer};
use crate::state::AppState;

fn project_root(state: &State<'_, AppState>) -> Option<String> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string())
}

/// The GitHub token the `gh` CLI is logged in with, for a GitHub MCP server's
/// env — so nobody pastes a token the machine already holds. `None` when gh
/// is absent or logged out. The value goes straight into the config the user
/// is editing; it is never logged.
#[tauri::command(async)]
pub fn gh_auth_token() -> Option<String> {
    let out = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}

#[tauri::command]
pub async fn mcp_list(state: State<'_, AppState>) -> AppResult<Vec<McpServer>> {
    let cwd = project_root(&state);
    tokio::task::spawn_blocking(move || mcp_service::list_servers(cwd.as_deref()))
        .await
        .map_err(|e| AppError::Other(format!("mcp list task panicked: {e}")))?
}

#[tauri::command]
pub async fn mcp_add(state: State<'_, AppState>, input: McpAddInput) -> AppResult<String> {
    let cwd = project_root(&state);
    tokio::task::spawn_blocking(move || mcp_service::add_server(&input, cwd.as_deref()))
        .await
        .map_err(|e| AppError::Other(format!("mcp add task panicked: {e}")))?
}

#[tauri::command]
pub async fn mcp_remove(
    state: State<'_, AppState>,
    name: String,
    scope: String,
) -> AppResult<String> {
    let cwd = project_root(&state);
    tokio::task::spawn_blocking(move || mcp_service::remove_server(&name, &scope, cwd.as_deref()))
        .await
        .map_err(|e| AppError::Other(format!("mcp remove task panicked: {e}")))?
}
