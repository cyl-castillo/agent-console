use std::path::PathBuf;

use tauri::State;

use crate::error::AppResult;
use crate::services::testigo_export::{self, ExportSummary};
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

/// Export a signed proof packet (DSSE-wrapped in-toto statement) for a case —
/// or the whole ledger — into `dest_dir` (defaults to `<project>/proofpacks`),
/// alongside the standalone HTML verifier.
#[tauri::command]
pub fn testigo_export(
    project_root: String,
    case_id: Option<String>,
    dest_dir: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ExportSummary> {
    let dest = dest_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&project_root).join("proofpacks"));
    testigo_export::export(&state.testigo, &project_root, case_id.as_deref(), &dest)
}

/// The signing key id + public key, for sharing out-of-band with receivers.
#[tauri::command]
pub fn testigo_public_key() -> AppResult<serde_json::Value> {
    testigo_export::public_key_info()
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
