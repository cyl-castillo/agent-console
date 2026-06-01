use std::path::Path;

use crate::error::AppResult;
use crate::services::usage_service::{self, UsageStats};

/// Token usage for a Claude session, read from its transcript. `None` when the
/// session has no transcript yet (or isn't a Claude session).
#[tauri::command]
pub fn session_usage(session_id: String, project_root: String) -> AppResult<Option<UsageStats>> {
    usage_service::read_usage(Path::new(&project_root), &session_id)
}
