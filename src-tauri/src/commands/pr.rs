use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use crate::commands::git::{current_repo, ledger_root};
use crate::error::{AppError, AppResult};
use crate::services::pr_service::{self, PrOverview};
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The case the branch's HEAD belongs to, for the PR lines of the ledger.
fn head_case(state: &AppState, root: &str, ov: &PrOverview) -> String {
    ov.evidence
        .commits
        .first()
        .and_then(|c| c.case_id.clone())
        .or_else(|| {
            current_repo(state)
                .ok()
                .and_then(|r| crate::services::git_service::head_sha(&r).ok())
                .and_then(|sha| state.testigo.case_for_commit_sha(root, &sha).ok().flatten())
        })
        .unwrap_or_else(|| "unbound".into())
}

/// F7: the checked-out branch's PR (through the user's own `gh`), its CI
/// and comments, and the ledger's evidence for every commit behind it.
/// The first sight of a (PR, head sha) pair is witnessed as `pr_link`.
#[tauri::command(async)]
pub fn pr_status(state: State<'_, AppState>) -> AppResult<PrOverview> {
    let repo = current_repo(&state)?;
    let root = ledger_root(&state)?;
    let events = state.testigo.list(&root, None, None)?;
    let ov = pr_service::overview(&repo, &events)?;
    if let Some(pr) = &ov.pr {
        let seen = events.iter().any(|e| {
            e.kind == "pr_link"
                && e.payload.get("number").and_then(Value::as_u64) == Some(pr.number)
                && e.payload.get("headSha").and_then(Value::as_str) == Some(pr.head_sha.as_str())
        });
        if !seen {
            let case = head_case(&state, &root, &ov);
            let _ = state.testigo.on_pr(
                &root,
                now_ms(),
                &case,
                "pr_link",
                "console",
                pr_service::link_payload(&ov),
            );
        }
    }
    Ok(ov)
}

/// F7: the review comments were typed into the agent as a prompt (the
/// human sends it); the ledger names which ones.
#[tauri::command(async)]
pub fn pr_autofix(number: u64, comment_ids: Vec<u64>, state: State<'_, AppState>) -> AppResult<()> {
    let root = ledger_root(&state)?;
    let repo = current_repo(&state)?;
    let head = crate::services::git_service::head_sha(&repo).unwrap_or_default();
    let case = state
        .testigo
        .case_for_commit_sha(&root, &head)?
        .unwrap_or_else(|| "unbound".into());
    state.testigo.on_pr(
        &root,
        now_ms(),
        &case,
        "pr_autofix",
        "human",
        json!({ "number": number, "headSha": head, "commentIds": comment_ids, "count": comment_ids.len() }),
    )?;
    Ok(())
}

/// F7: merge when green. The gate is evidence — every turn behind the
/// branch verified (or accepted by a human) — or an explicit human word
/// with a reason; nothing in between. The decision is witnessed before
/// `gh` runs, so a failed merge still shows who decided what.
#[tauri::command(async)]
pub fn pr_merge(
    number: u64,
    gate: String,
    reason: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let repo = current_repo(&state)?;
    let root = ledger_root(&state)?;
    let events = state.testigo.list(&root, None, None)?;
    let ov = pr_service::overview(&repo, &events)?;
    let pr = ov.pr.as_ref().ok_or_else(|| {
        AppError::InvalidArgument("no pull request for the checked-out branch".into())
    })?;
    if pr.number != number {
        return Err(AppError::InvalidArgument(format!(
            "the checked-out branch's PR is #{}, not #{number}",
            pr.number
        )));
    }
    let reason = reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    match gate.as_str() {
        "verified" if ov.evidence.all_verified => {}
        "verified" => return Err(AppError::InvalidArgument(
            "not every turn behind this branch is verified — merging needs your word and a reason"
                .into(),
        )),
        "human" if reason.is_some() => {}
        "human" => {
            return Err(AppError::InvalidArgument(
                "merging on your word needs a reason".into(),
            ))
        }
        other => {
            return Err(AppError::InvalidArgument(format!(
                "unknown merge gate `{other}` (verified | human)"
            )))
        }
    }
    let case = head_case(&state, &root, &ov);
    let green = pr.ci == "green";
    state.testigo.on_pr(
        &root,
        now_ms(),
        &case,
        "pr_decision",
        "human",
        json!({
            "number": pr.number,
            "url": pr.url,
            "headSha": pr.head_sha,
            "decision": if green { "merge" } else { "merge_when_green" },
            "gate": gate,
            "reason": reason,
            "ci": pr.ci,
            "evidence": {
                "commits": ov.evidence.commits.len(),
                "verified": ov.evidence.verified,
                "accepted": ov.evidence.accepted,
                "unverified": ov.evidence.unverified,
                "contradicted": ov.evidence.contradicted,
                "unbound": ov.evidence.unbound,
            },
        }),
    )?;
    let out = pr_service::merge(&repo, number, green)?;
    let _ = app.emit(
        "console://pr",
        &json!({ "project": root, "number": number, "merged": green }),
    );
    Ok(out)
}
