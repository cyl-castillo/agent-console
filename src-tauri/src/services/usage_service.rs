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
