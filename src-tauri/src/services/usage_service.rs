//! Reads token usage for a Claude Code session from its transcript.
//!
//! Claude Code appends one JSON object per line to
//! `~/.claude/projects/<slug>/<session-id>.jsonl`, where assistant turns carry
//! a `message.usage` block (`input_tokens`, `output_tokens`,
//! `cache_read_input_tokens`, `cache_creation_input_tokens`). We aggregate
//! those so the status bar can show how much of the model context is in use.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// Standard context window for current Claude models (Opus/Sonnet).
const CONTEXT_WINDOW: u64 = 200_000;
/// Long-context tier. The transcript doesn't record the active limit, so when a
/// turn's footprint exceeds the standard window we assume the session is on the
/// 1M tier and switch the denominator — otherwise the indicator would read
/// >100% nonsensically.
const CONTEXT_WINDOW_LONG: u64 = 1_000_000;

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    /// Tokens occupying the model context as of the latest assistant turn:
    /// `input + cache_read + cache_creation` of the most recent usage block.
    pub context_tokens: u64,
    pub input_total: u64,
    pub output_total: u64,
    pub cache_read_total: u64,
    pub cache_creation_total: u64,
    /// Nominal model context window (tokens).
    pub context_window: u64,
}

/// Path to the transcript for `session_id` under `project_root`. Mirrors the
/// slug scheme Claude Code uses (and `context_service::memory_dir_for`):
/// each path separator becomes `-`.
fn transcript_path(project_root: &Path, session_id: &str) -> AppResult<PathBuf> {
    let abs = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let slug = abs.to_string_lossy().replace(['/', '\\'], "-");
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    Ok(home
        .join(".claude")
        .join("projects")
        .join(slug)
        .join(format!("{session_id}.jsonl")))
}

/// Aggregate token usage for a Claude session. Returns `None` when there is no
/// transcript yet (brand-new session) or it carries no usage (e.g. a non-Claude
/// agent), so the caller can simply hide the indicator.
pub fn read_usage(project_root: &Path, session_id: &str) -> AppResult<Option<UsageStats>> {
    let path = transcript_path(project_root, session_id)?;
    if !path.exists() {
        return Ok(None);
    }

    let reader = BufReader::new(fs::File::open(&path)?);
    let mut stats = UsageStats::default();
    let mut saw_usage = false;

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        // Cheap pre-filter: skip the many lines that carry no usage block.
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(u) = v
            .get("message")
            .and_then(|m| m.get("usage"))
            .or_else(|| v.get("usage"))
        else {
            continue;
        };

        let field = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let input = field("input_tokens");
        let output = field("output_tokens");
        let cache_read = field("cache_read_input_tokens");
        let cache_creation = field("cache_creation_input_tokens");

        // Some lines include a "usage" key with no real token counts; ignore them
        // so they don't reset the latest-context figure to zero.
        if input == 0 && output == 0 && cache_read == 0 && cache_creation == 0 {
            continue;
        }

        saw_usage = true;
        stats.input_total += input;
        stats.output_total += output;
        stats.cache_read_total += cache_read;
        stats.cache_creation_total += cache_creation;
        // Context reflects the *latest* turn, so overwrite rather than sum.
        stats.context_tokens = input + cache_read + cache_creation;
    }

    // Pick the denominator from the observed footprint: a turn larger than the
    // standard window means the session is on the long-context tier.
    stats.context_window = if stats.context_tokens > CONTEXT_WINDOW {
        CONTEXT_WINDOW_LONG
    } else {
        CONTEXT_WINDOW
    };

    Ok(saw_usage.then_some(stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test fn — it mutates the process-global HOME so the transcript is
    /// read from a sandbox instead of the developer's real ~/.claude.
    #[test]
    fn read_usage_aggregates_totals_and_tracks_latest_context() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fake_home =
            std::env::temp_dir().join(format!("ac-usage-home-{}-{nanos}", std::process::id()));
        let project =
            std::env::temp_dir().join(format!("ac-usage-proj-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&fake_home).unwrap();
        fs::create_dir_all(&project).unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        let run = || {
            // No transcript yet → None, indicator hidden.
            assert!(read_usage(&project, "sess-1").unwrap().is_none());

            let canon = project.canonicalize().unwrap();
            let slug = canon.to_string_lossy().replace(['/', '\\'], "-");
            let dir = fake_home.join(".claude").join("projects").join(&slug);
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("sess-1.jsonl");

            // A transcript with: garbage, a usage-shaped line with all zeros
            // (must not reset context), and two real assistant turns.
            let lines = [
                "not json at all",
                r#"{"type":"other","usage":{"input_tokens":0,"output_tokens":0}}"#,
                r#"{"message":{"usage":{"input_tokens":1000,"output_tokens":50,"cache_read_input_tokens":200,"cache_creation_input_tokens":30}}}"#,
                r#"{"message":{"usage":{"input_tokens":2000,"output_tokens":80,"cache_read_input_tokens":500,"cache_creation_input_tokens":0}}}"#,
            ];
            fs::write(&path, lines.join("\n")).unwrap();

            let stats = read_usage(&project, "sess-1").unwrap().expect("some usage");
            assert_eq!(stats.input_total, 3000);
            assert_eq!(stats.output_total, 130);
            assert_eq!(stats.cache_read_total, 700);
            assert_eq!(stats.cache_creation_total, 30);
            // Context is the LATEST turn's footprint, not the sum.
            assert_eq!(stats.context_tokens, 2500);
            assert_eq!(stats.context_window, 200_000);

            // A transcript whose lines carry no usage at all → None.
            fs::write(&path, "{\"type\":\"user\"}\n").unwrap();
            assert!(read_usage(&project, "sess-1").unwrap().is_none());

            // A turn bigger than the standard window flips the denominator to
            // the long-context tier (otherwise the indicator reads >100%).
            let big = r#"{"message":{"usage":{"input_tokens":250000,"output_tokens":10,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
            fs::write(&path, big).unwrap();
            let stats = read_usage(&project, "sess-1").unwrap().unwrap();
            assert_eq!(stats.context_window, 1_000_000);
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));

        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        fs::remove_dir_all(&fake_home).ok();
        fs::remove_dir_all(&project).ok();
        if let Err(p) = result {
            std::panic::resume_unwind(p);
        }
    }
}
