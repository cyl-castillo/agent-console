use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::git_service::{self, BranchInfo, GitCommitInfo, GitStatus};
use crate::state::AppState;

/// The checkout git commands operate on: the active session's isolated
/// worktree when one is active (set via `set_active_repo`), else the project
/// root. This is what makes the Changes view follow the active session.
fn current_repo(state: &AppState) -> AppResult<std::path::PathBuf> {
    let s = state.inner.lock();
    if let Some(wt) = &s.active_repo {
        return Ok(wt.clone());
    }
    s.project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))
}

#[tauri::command(async)]
pub fn git_status(state: State<'_, AppState>) -> AppResult<GitStatus> {
    let repo = current_repo(&state)?;
    git_service::status(&repo)
}

#[tauri::command(async)]
pub fn git_diff_file(file: String, state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    git_service::diff_file(&repo, &file)
}

#[tauri::command(async)]
pub fn git_revert_file(file: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = current_repo(&state)?;
    git_service::revert_file(&repo, &file)
}

#[tauri::command(async)]
pub fn git_stage_file(file: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = current_repo(&state)?;
    git_service::stage_file(&repo, &file)
}

#[tauri::command(async)]
pub fn git_unstage_file(file: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = current_repo(&state)?;
    git_service::unstage_file(&repo, &file)
}

/// Turns closed more than this long ago don't stamp commits — the trailer is
/// for "commit the work the agent just did", not archaeology.
const TESTIGO_TRAILER_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;

/// Ledger line for a commit the human just made (T4b): bound to the turn
/// whose diff produced the staged files. Best-effort, never blocks the
/// commit; witness-off projects simply record nothing.
/// The Testigo ledger for the active checkout — the same rule hook events are
/// filed by (`ledger_root_for_cwd`), so a commit lands in the ledger that holds
/// the turn that produced it, whether the session runs in the project root, a
/// worktree or a linked folder.
fn ledger_root(state: &AppState) -> AppResult<String> {
    let repo = current_repo(state)?;
    let open = state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string());
    Ok(crate::services::hooks_service::ledger_root_for_cwd(
        &repo.to_string_lossy(),
        open.as_deref(),
    ))
}

fn record_commit(state: &AppState, sha: &str, message: &str, files: &[String], amend: bool) {
    let Ok(root) = ledger_root(state) else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let _ = state.testigo.on_commit(
        &root,
        now,
        sha,
        message,
        files,
        amend,
        TESTIGO_TRAILER_MAX_AGE_MS,
    );
}

#[tauri::command(async)]
pub fn git_commit(message: String, state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    let staged_for_ledger = git_service::staged_files(&repo).unwrap_or_default();
    // Testigo trailer: stamp the commit with the case whose recorded turn
    // produced the staged files (ledger evidence, not active-session
    // guessing). Best-effort — a ledger miss never blocks the commit.
    let mut message = message;
    if !message.contains("Testigo-Case:") {
        // Trailers are repo marks: per-ledger opt-in, off by default.
        let root = ledger_root(&state)
            .ok()
            .filter(|r| state.testigo.repo_marks(r));
        if let Some(root) = root {
            let staged = git_service::staged_files(&repo).unwrap_or_default();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if let Ok(Some(case)) =
                state
                    .testigo
                    .case_for_files(&root, &staged, now, TESTIGO_TRAILER_MAX_AGE_MS)
            {
                message = format!("{}\n\nTestigo-Case: {case}", message.trim_end());
                // V2-A: carry the ledger head into pushed history — the
                // distributed half of the anchor (the local half lives in
                // refs/agent-console/testigo-head).
                if let Ok(Some((seq, hash))) = state.testigo.head(&root) {
                    message = format!("{message}\nTestigo-Head: {seq}:{hash}");
                }
            }
        }
    }
    let sha = git_service::commit(&repo, &message)?;
    record_commit(&state, &sha, &message, &staged_for_ledger, false);
    Ok(sha)
}

/// Push the current branch (creating its upstream if needed) and hand back
/// the PR/MR link for it. P1: the loop no longer ends at `commit`.
#[tauri::command(async)]
pub fn git_push(state: State<'_, AppState>) -> AppResult<git_service::PushResult> {
    let repo = current_repo(&state)?;
    git_service::push_current(&repo)
}

/// The PR/MR link for the current branch as pushed (None: no upstream,
/// default branch, or an unrecognized host).
#[tauri::command(async)]
pub fn git_pr_url(state: State<'_, AppState>) -> AppResult<Option<String>> {
    let repo = current_repo(&state)?;
    git_service::pr_url_current(&repo)
}

/// What `git_attach_proof` put on the branch.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachProofSummary {
    pub case_id: String,
    /// Repo-relative path of the committed packet.
    pub path: String,
    pub commit_sha: String,
    pub event_count: usize,
    pub redaction_count: usize,
    /// The packet's `gitCommit` subject (the case's last recorded commit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
}

/// P2 proof-on-PR: export the packet for the case that produced HEAD and
/// commit it under `.testigo/proofs/`, so the PR carries its own evidence
/// and CI can verify it (signature, chain, diff coverage). The commit
/// touches ONLY the packet file — whatever the user staged stays staged.
/// Auto-redaction applies as in any export; manual redaction stays in the
/// Proof panel flow, and the packet lands in the PR diff where it can be
/// reviewed before merge.
#[tauri::command(async)]
pub fn git_attach_proof(state: State<'_, AppState>) -> AppResult<AttachProofSummary> {
    let repo = current_repo(&state)?;
    let root = ledger_root(&state)?;
    let head = git_service::head_sha(&repo)?;
    let case = state
        .testigo
        .case_for_commit_sha(&root, &head)?
        .ok_or_else(|| {
            AppError::InvalidArgument(
                "HEAD has no case in the ledger — commit from the console first so the evidence binds to this branch".into(),
            )
        })?;
    // Export into a scratch dir first: the repo gets ONLY the packet, not
    // the standalone verifier HTML the export drops next to it.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("ac-attach-{}-{nanos}", std::process::id()));
    let summary =
        crate::services::testigo_export::export(&state.testigo, &root, Some(&case), &tmp, &[])?;
    let file_name = std::path::Path::new(&summary.path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| AppError::Other("packet path has no file name".into()))?;
    let rel = format!(".testigo/proofs/{file_name}");
    let dest_dir = repo.join(".testigo").join("proofs");
    std::fs::create_dir_all(&dest_dir)?;
    std::fs::copy(&summary.path, dest_dir.join(&file_name))?;
    let _ = std::fs::remove_dir_all(&tmp);
    let msg = format!("Attach proof packet ({case})");
    let sha = git_service::commit_paths(&repo, &msg, std::slice::from_ref(&rel))?;
    record_commit(&state, &sha, &msg, std::slice::from_ref(&rel), false);
    Ok(AttachProofSummary {
        case_id: case,
        path: rel,
        commit_sha: sha,
        event_count: summary.event_count,
        redaction_count: summary.redaction_count,
        git_commit: summary.git_commit,
    })
}

#[tauri::command(async)]
pub fn git_branches(state: State<'_, AppState>) -> AppResult<Vec<BranchInfo>> {
    let repo = current_repo(&state)?;
    git_service::branches(&repo)
}

#[tauri::command(async)]
pub fn git_checkout_branch(name: String, state: State<'_, AppState>) -> AppResult<()> {
    let repo = current_repo(&state)?;
    git_service::checkout_branch(&repo, &name)
}

#[tauri::command(async)]
pub fn git_recent_messages(
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let repo = current_repo(&state)?;
    git_service::recent_messages(&repo, limit.unwrap_or(10))
}

#[tauri::command(async)]
pub fn git_head_message(state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    git_service::head_message(&repo)
}

#[tauri::command(async)]
pub fn git_amend_commit(message: String, state: State<'_, AppState>) -> AppResult<String> {
    let repo = current_repo(&state)?;
    let staged_for_ledger = git_service::staged_files(&repo).unwrap_or_default();
    let sha = git_service::amend_commit(&repo, &message)?;
    record_commit(&state, &sha, &message, &staged_for_ledger, true);
    Ok(sha)
}

#[tauri::command(async)]
pub fn git_file_log(
    file: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<Vec<GitCommitInfo>> {
    let repo = current_repo(&state)?;
    git_service::file_log(&repo, &file, limit.unwrap_or(5))
}
