use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::mcp_service::{self, McpAddInput, McpServer};
use crate::state::AppState;

fn project_root(state: &State<'_, AppState>) -> Option<String> {
    state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string())
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
