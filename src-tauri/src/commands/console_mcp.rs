use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// The webview's answer to a `console://call` roundtrip.
#[tauri::command(async)]
pub fn console_reply(state: State<'_, AppState>, id: u64, result: Value) {
    state.console_mcp.reply(id, result);
}

/// What the user pastes into their agent to point it at this console.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleMcpConfig {
    /// The hook-bridge sidecar the server runs as.
    pub command: String,
    pub args: Vec<String>,
    /// `claude mcp add …` one-liner.
    pub claude_add: String,
    /// `codex -c …` override / `~/.codex/config.toml` snippet.
    pub codex_toml: String,
    /// The tools the server offers, for the panel's listing.
    pub tools: Vec<String>,
}

#[tauri::command(async)]
pub fn console_mcp_config(
    state: State<'_, AppState>,
    caller: Option<String>,
) -> AppResult<ConsoleMcpConfig> {
    let project = state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string())
        .ok_or_else(|| AppError::Other("No project open".into()))?;
    let bridge = state
        .hooks
        .bridge_binary()
        .ok_or_else(|| AppError::Other("The hook bridge sidecar is not installed".into()))?
        .to_string_lossy()
        .to_string();
    let caller = caller
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| "agent".into());
    let args = vec![
        "console-mcp".to_string(),
        "--project".into(),
        project.clone(),
        "--caller".into(),
        caller.clone(),
    ];
    let sh = |s: &str| {
        if s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:".contains(c))
        {
            s.to_string()
        } else {
            format!("'{}'", s.replace('\'', "'\\''"))
        }
    };
    let claude_add = format!(
        "claude mcp add agent-console -- {} {}",
        sh(&bridge),
        args.iter().map(|a| sh(a)).collect::<Vec<_>>().join(" ")
    );
    let quoted = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    let codex_toml = format!(
        "[mcp_servers.agent_console]\ncommand = {}\nargs = [{}]",
        quoted(&bridge),
        args.iter()
            .map(|a| quoted(a))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(ConsoleMcpConfig {
        command: bridge,
        args,
        claude_add,
        codex_toml,
        tools: crate::services::console_mcp::TOOL_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect(),
    })
}
