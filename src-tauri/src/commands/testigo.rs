use tauri::State;

use crate::error::AppResult;
use crate::services::testigo_service::{ProofEvent, VerifyReport};
use crate::state::AppState;

#[tauri::command]
pub fn testigo_list(
    project_root: String,
    case_id: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> AppResult<Vec<ProofEvent>> {
    state
        .testigo
        .list(&project_root, case_id.as_deref(), limit)
}

#[tauri::command]
pub fn testigo_verify(project_root: String, state: State<'_, AppState>) -> AppResult<VerifyReport> {
    state.testigo.verify(&project_root)
}

/// Bind a terminal session to a named case — called by the frontend when a
/// session is seeded from a Jira ticket, so the ticket→session lineage is
/// part of the evidence chain.
#[tauri::command]
pub fn testigo_link_case(
    project_root: String,
    term_id: String,
    ticket: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    state
        .testigo
        .link_case(&project_root, ts, &term_id, &format!("jira:{ticket}"))?;
    Ok(())
}
