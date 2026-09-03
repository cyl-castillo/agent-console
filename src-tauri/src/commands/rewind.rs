use std::path::PathBuf;

use serde::Serialize;
use serde_json::json;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::{rewind_service, snapshot_service};
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRewindResult {
    /// Pre-restore backup of the tree that was about to be overwritten (same
    /// contract as `snapshot_restore`) — lets the UI offer an undo.
    pub backup_sha: Option<String>,
    /// The forked transcript's session id — what the relaunched terminal
    /// resumes. None = the fork failed and only the files were restored.
    pub fork_session_id: Option<String>,
    /// Why the fork was not attempted / failed. The UI must SAY this: files
    /// restored with the agent still remembering later turns is exactly the
    /// desync this feature exists to fix, so degrading silently would lie.
    pub fork_error: Option<String>,
}

/// "Rewind to turn N": restore the working tree to the turn's post-snapshot
/// AND fork the Claude conversation truncated after that turn, so code and
/// context rewind together. The file restore is the core action and happens
/// first (with a pre-restore backup); the transcript fork is best-effort on
/// top and degrades honestly into `fork_error`. The original transcript is
/// never mutated — the fork is a new file under a new uuid.
#[tauri::command]
pub fn turn_rewind(
    repo: Option<String>,
    commit_sha: String,
    session_id: String,
    cutoff_ms: i64,
    term_id: Option<String>,
    turn_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<TurnRewindResult> {
    // The id comes from ledger events (world-writable state) and becomes a
    // filename + a `--resume` argument: validate at the boundary, like the
    // TS side's isSafeSessionId.
    if !rewind_service::is_safe_session_id(&session_id) {
        return Err(AppError::InvalidArgument("unsafe session id".into()));
    }

    // The checkout the TURN ran in (worktree sessions differ from the project
    // root); the global active repo is only the fallback for old ledger
    // events that carried no cwd.
    let repo_path = match repo.as_deref().filter(|r| !r.is_empty()) {
        Some(r) => {
            let p = PathBuf::from(r);
            if !p.is_dir() {
                return Err(AppError::NotADirectory(r.into()));
            }
            p
        }
        None => active_repo(&state)?,
    };
    let project_root = state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string());

    // 1+2. Backup, then restore — same contract as `snapshot_restore`: the
    // backup is best-effort and never blocks, the restore failing aborts the
    // whole rewind (nothing else has happened yet).
    let backup_sha = snapshot_service::create(&repo_path, &format!("pre-restore-{}", now_nanos()))
        .ok()
        .flatten()
        .map(|s| s.commit_sha);
    snapshot_service::restore(&repo_path, &commit_sha)?;

    // 3. Fork the transcript, gated on `claude --version` (the trust marker
    // is broken — W5 — so hooks say nothing about what the CLI supports).
    let fork = rewind_service::fork_gate().and_then(|()| {
        rewind_service::fork_transcript(&session_id, cutoff_ms).map_err(|e| e.to_string())
    });
    let (fork_session_id, fork_truncated, fork_error) = match fork {
        Ok(out) => (Some(out.fork_session_id), Some(out.truncated), None),
        Err(reason) => (None, None, Some(reason)),
    };

    // 4. Ledger: a rewind rewrites the checkout — that is evidence. Same
    // best-effort contract as the hook call sites: witness-off projects err
    // by design, only real failures get logged.
    if let Some(root) = project_root {
        if let Err(e) = state.testigo.on_rewind(
            &root,
            now_ms(),
            term_id.as_deref(),
            Some(&session_id),
            turn_id,
            json!({
                "restoredSha": commit_sha,
                "backupSha": backup_sha,
                "forkSessionId": fork_session_id,
                // false = the rewound-to turn was already the last one and the
                // fork is a full copy; useful when auditing what was dropped.
                "forkTruncated": fork_truncated,
                "forkError": fork_error,
                "cwd": repo_path.to_string_lossy(),
            }),
        ) {
            let msg = e.to_string();
            if !msg.contains("witnessing disabled") {
                eprintln!("rewind: testigo append failed: {msg}");
            }
        }
    }

    Ok(TurnRewindResult {
        backup_sha,
        fork_session_id,
        fork_error,
    })
}

/// Same resolution as `commands::snapshot::repo`: the active worktree when
/// set, else the project root.
fn active_repo(state: &AppState) -> AppResult<PathBuf> {
    let s = state.inner.lock();
    if let Some(wt) = &s.active_repo {
        return Ok(wt.clone());
    }
    s.project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))
}

fn now_nanos() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
