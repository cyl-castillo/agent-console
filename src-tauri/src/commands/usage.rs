use std::path::Path;

use crate::error::AppResult;
use crate::services::usage_service::{self, UsageStats};

/// Token usage for the active agent session, read from its on-disk transcript:
/// Claude's `~/.claude/projects/<slug>/<id>.jsonl` or Codex's rollout under
/// `~/.codex/sessions/`. `None` when no transcript exists yet (brand-new
/// session) or it carries no usage, so the caller can simply hide the pill.
#[tauri::command]
pub fn session_usage(
    session_id: String,
    project_root: String,
    agent: Option<String>,
) -> AppResult<Option<UsageStats>> {
    match agent.as_deref() {
        Some("codex") => usage_service::read_codex_usage(&session_id),
        _ => usage_service::read_usage(Path::new(&project_root), &session_id),
    }
}
