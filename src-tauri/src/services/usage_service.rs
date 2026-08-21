//! Reads token usage for an agent session from its on-disk transcript.
//!
//! Claude Code appends one JSON object per line to
//! `~/.claude/projects/<slug>/<session-id>.jsonl`, where assistant turns carry
//! a `message.usage` block (`input_tokens`, `output_tokens`,
//! `cache_read_input_tokens`, `cache_creation_input_tokens`). We aggregate
//! those so the status bar can show how much of the model context is in use.
//!
//! Codex writes the equivalent rollout to
//! `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-<timestamp>-<session-id>.jsonl`,
//! whose `token_count` events carry cumulative + last-request token usage and
//! the model context window — no `codex app-server` daemon needed to read it.

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

/// Fallback context window for Codex when a rollout's `token_count` events omit
/// `model_context_window` (observed value for GPT-5.x; virtually every event
/// carries the real figure, so this rarely decides anything).
const CODEX_CONTEXT_WINDOW_FALLBACK: u64 = 258_400;

/// Locate the Codex rollout for `session_id`. Rollouts live under
/// `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-<timestamp>-<session-id>.jsonl`;
/// the date prefix isn't derivable from the id, so walk the (shallow, 3-level)
/// date tree newest-first and stop at the first filename match.
fn find_codex_rollout(session_id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let root = home.join(".codex").join("sessions");
    let suffix = format!("-{session_id}.jsonl");

    // read_dir order is arbitrary — sort descending so recent sessions (the
    // ones the status bar actually polls) are found in the first few dirs.
    let sorted_desc = |dir: &Path| -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        entries.sort();
        entries.reverse();
        entries
    };

    for year in sorted_desc(&root) {
        for month in sorted_desc(&year) {
            for day in sorted_desc(&month) {
                for file in sorted_desc(&day) {
                    let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    if name.starts_with("rollout-") && name.ends_with(&suffix) {
                        return Some(file);
                    }
                }
            }
        }
    }
    None
}

/// Token usage for a Codex session, read from its rollout file. `token_count`
/// events already carry cumulative totals (`total_token_usage`) plus the last
/// request's footprint (`last_token_usage`), so the last event wins outright —
/// nothing to sum. Returns `None` when no rollout exists for the id or it has
/// no usage events yet.
pub fn read_codex_usage(session_id: &str) -> AppResult<Option<UsageStats>> {
    let Some(path) = find_codex_rollout(session_id) else {
        return Ok(None);
    };

    let reader = BufReader::new(fs::File::open(&path)?);
    let mut stats: Option<UsageStats> = None;

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        // Cheap pre-filter: rollouts are mostly turn payloads without usage.
        if !line.contains("\"token_count\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let payload = v.get("payload").unwrap_or(&v);
        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }
        // `info` is null on housekeeping events (e.g. rate-limit-only updates);
        // those must not clobber the last real reading.
        let Some(info) = payload.get("info").filter(|i| !i.is_null()) else {
            continue;
        };

        let usage_field = |block: &str, k: &str| {
            info.get(block)
                .and_then(|b| b.get(k))
                .and_then(|x| x.as_u64())
                .unwrap_or(0)
        };
        let last_input = usage_field("last_token_usage", "input_tokens");
        let last_output = usage_field("last_token_usage", "output_tokens");

        stats = Some(UsageStats {
            // Codex's `input_tokens` already includes the cached share, so the
            // last request's input+output is the current context footprint.
            context_tokens: last_input + last_output,
            input_total: usage_field("total_token_usage", "input_tokens"),
            output_total: usage_field("total_token_usage", "output_tokens"),
            cache_read_total: usage_field("total_token_usage", "cached_input_tokens"),
            // Codex doesn't report cache writes separately.
            cache_creation_total: 0,
            context_window: info
                .get("model_context_window")
                .and_then(|w| w.as_u64())
                .unwrap_or(CODEX_CONTEXT_WINDOW_FALLBACK),
        });
    }

    Ok(stats.filter(|s| s.context_tokens > 0 || s.output_total > 0))
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

    /// Codex rollout reading: locates the file by id under the date tree, takes
    /// the LAST token_count event with real info (cumulative totals — no
    /// summing), and ignores null-info housekeeping events.
    #[test]
    fn read_codex_usage_takes_last_token_count_event() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fake_home =
            std::env::temp_dir().join(format!("ac-codex-home-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&fake_home).unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        let run = || {
            // No rollout anywhere → None, indicator hidden.
            assert!(read_codex_usage("019e-abc").unwrap().is_none());

            let day = fake_home
                .join(".codex")
                .join("sessions")
                .join("2026")
                .join("08")
                .join("21");
            fs::create_dir_all(&day).unwrap();
            let path = day.join("rollout-2026-08-21T10-00-00-019e-abc.jsonl");

            // A rollout with: session meta, a first token_count, a null-info
            // token_count (must not clobber), and the latest real reading.
            let lines = [
                r#"{"type":"session_meta","payload":{"id":"019e-abc"}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":300,"output_tokens":50},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":300,"output_tokens":50},"model_context_window":258400}}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{}}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":9000,"cached_input_tokens":4000,"output_tokens":700},"last_token_usage":{"input_tokens":8000,"cached_input_tokens":3700,"output_tokens":650},"model_context_window":258400}}}"#,
            ];
            fs::write(&path, lines.join("\n")).unwrap();

            let stats = read_codex_usage("019e-abc").unwrap().expect("some usage");
            // Totals come from the LAST event's cumulative block, not a sum.
            assert_eq!(stats.input_total, 9000);
            assert_eq!(stats.output_total, 700);
            assert_eq!(stats.cache_read_total, 4000);
            assert_eq!(stats.cache_creation_total, 0);
            // Context = last request's input (cached included) + output.
            assert_eq!(stats.context_tokens, 8650);
            assert_eq!(stats.context_window, 258_400);

            // A different id doesn't match this rollout.
            assert!(read_codex_usage("other-id").unwrap().is_none());
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));

        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        fs::remove_dir_all(&fake_home).ok();
        if let Err(p) = result {
            std::panic::resume_unwind(p);
        }
    }
}
