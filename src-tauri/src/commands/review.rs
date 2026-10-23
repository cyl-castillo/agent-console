use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::services::reviewer_service::{self, ReviewVerdict};
use crate::services::testigo_service::ProofEvent;
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// D10: ask the reviewer agent to review a finished turn (the session's last
/// one when `turn_id` is omitted). Blocking (a headless engine run); the
/// verdict is witnessed and also emitted as `console://review` for the Inbox.
#[tauri::command(async)]
pub fn review_turn(
    project_root: String,
    term_id: String,
    turn_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<ReviewVerdict> {
    let verdict = reviewer_service::review(
        &state.testigo,
        &project_root,
        &term_id,
        turn_id.as_deref(),
        now_ms(),
    )?;
    let _ = app.emit(
        "console://review",
        &json!({ "project": project_root, "verdict": verdict }),
    );
    Ok(verdict)
}

/// The human's answer to a verdict: `accept` (the block stands, nothing
/// merges until the finding is addressed) or `override` (with a reason —
/// the proof-on-PR check honours an override, never a silent dismissal).
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn review_decide(
    project_root: String,
    term_id: String,
    turn_id: String,
    case_id: String,
    verdict_seq: u64,
    decision: String,
    reason: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ProofEvent> {
    if decision != "accept" && decision != "override" {
        return Err(AppError::Other(
            "decision must be accept or override".into(),
        ));
    }
    if decision == "override" && reason.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(AppError::Other("An override needs a reason".into()));
    }
    state.testigo.on_review(
        &project_root,
        now_ms(),
        &case_id,
        Some(&turn_id),
        &term_id,
        "review_decision",
        "human",
        json!({ "decision": decision, "reason": reason, "verdictSeq": verdict_seq }),
    )
}

/// F1d: the human's answer to the console's own `verification` verdict on a
/// turn: `accept` (the claim stands on the human's word — with a reason
/// when the evidence contradicted it; the PR gate then warns instead of
/// failing) or `evidence_requested` (the human asked the agent to run the
/// gate; the next turn's verdict answers). Both witnessed as
/// `verification_decision`, pinned to the turn.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn verification_decide(
    project_root: String,
    term_id: String,
    turn_id: String,
    case_id: String,
    verification_seq: u64,
    decision: String,
    reason: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ProofEvent> {
    if decision != "accept" && decision != "evidence_requested" {
        return Err(AppError::Other(
            "decision must be accept or evidence_requested".into(),
        ));
    }
    let reason = reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    state.testigo.on_review(
        &project_root,
        now_ms(),
        &case_id,
        Some(&turn_id),
        &term_id,
        "verification_decision",
        "human",
        json!({ "decision": decision, "reason": reason, "verificationSeq": verification_seq }),
    )
}
