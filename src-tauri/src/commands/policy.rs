use std::path::Path;

use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::services::policy_service::{self, Policy, PolicyStatus};
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// F6: the project's approval policy as the file holds it, its hash and
/// what it compiles to for each engine.
#[tauri::command(async)]
pub fn policy_status(project_root: String) -> AppResult<PolicyStatus> {
    policy_service::status(Path::new(&project_root))
}

/// F6: write the policy file from its JSON text (validated; the file is the
/// source of truth and lives in the repo, so it is versioned with the code).
#[tauri::command(async)]
pub fn policy_save(project_root: String, policy_json: String) -> AppResult<PolicyStatus> {
    let policy: Policy = serde_json::from_str(&policy_json)
        .map_err(|e| AppError::InvalidArgument(format!("policy is not valid JSON: {e}")))?;
    policy_service::save(Path::new(&project_root), &policy)
}

/// F6: compile the policy into the three engines' files and witness
/// `policy_set` with the hash every later approval will carry.
#[tauri::command(async)]
pub fn policy_apply(
    project_root: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<PolicyStatus> {
    let root = Path::new(&project_root);
    let status = policy_service::status(root)?;
    if !status.errors.is_empty() {
        return Err(AppError::InvalidArgument(status.errors.join("; ")));
    }
    let written = policy_service::apply(root, &status.compiled)?;
    let _ = state.testigo.on_policy_set(
        &project_root,
        now_ms(),
        &status.hash,
        json!({
            "rules": status.policy.rules.len(),
            "claude": status.compiled.claude.allow.len() + status.compiled.claude.ask.len() + status.compiled.claude.deny.len(),
            "codex": status.compiled.codex_rules.len(),
            "opencode": status.compiled.opencode_permission.as_object().map(|o| o.len()).unwrap_or(0),
            "skipped": status.compiled.skipped.len(),
            "files": written,
        }),
    );
    let _ = app.emit(
        "console://policy",
        &json!({ "project": project_root, "hash": status.hash }),
    );
    Ok(status)
}
