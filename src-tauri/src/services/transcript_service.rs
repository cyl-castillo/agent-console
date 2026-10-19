//! F5: the agent's words as they are written, not when the turn closes.
//!
//! Claude appends every assistant content block to
//! `~/.claude/projects/<slug>/<session>.jsonl` the moment it has it; Codex
//! appends `response_item`s to its rollout under `~/.codex/sessions/`. The
//! webview asks to follow the transcript of the session it is showing and the
//! watcher tails that file, emitting `transcript://delta` per batch of new
//! lines. This is presentation: the ledger stays the truth about what a turn
//! did; the transcript is what the agent said while doing it.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};
use crate::services::hooks_service::{await_wake, dir_wake_source, drain_new_lines};
use crate::services::rewind_service::{is_safe_session_id, locate_transcript};
use crate::services::usage_service::find_codex_rollout;

/// How much of an existing transcript is read on first open: enough to show
/// the turn in flight when the human switches to a session mid-turn, not the
/// whole history (that is the ledger's job, and files reach hundreds of MB).
const BACKFILL_BYTES: u64 = 512 * 1024;
/// Tool input is a glance in the conversation, not the full argument.
const INPUT_EXCERPT_CHARS: usize = 160;
/// Poll cadence while the transcript file does not exist yet (a brand-new
/// session writes it on the first prompt).
const LOCATE_RETRY: Duration = Duration::from_secs(1);

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptItem {
    /// A paragraph the agent wrote (one content block).
    Text { id: String, ts: i64, text: String },
    /// The agent called a tool; `input` is an excerpt (the command, the path).
    ToolUse {
        id: String,
        ts: i64,
        name: String,
        input: String,
    },
    /// The tool answered; `id` pairs it with its `ToolUse`.
    ToolResult { id: String, ts: i64, ok: bool },
    /// A human prompt — the boundary between turns inside the stream.
    User { id: String, ts: i64, text: String },
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptDelta {
    pub session_id: String,
    pub engine: String,
    /// True for the first batch of an existing file: the tail since the last
    /// prompt, replacing whatever the webview held for this session.
    pub backfill: bool,
    pub items: Vec<TranscriptItem>,
}

/// One tail thread per followed session; dropping the flag stops it.
#[derive(Default)]
pub struct TranscriptWatchers {
    inner: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl TranscriptWatchers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn watch(&self, app: AppHandle, session_id: &str, engine: &str) -> AppResult<()> {
        if !is_safe_session_id(session_id) {
            return Err(AppError::Other(format!("unsafe session id: {session_id}")));
        }
        let engine = match engine {
            "claude" | "codex" => engine.to_string(),
            other => return Err(AppError::Other(format!("no transcript for engine {other}"))),
        };
        let mut map = self.inner.lock();
        if map.contains_key(session_id) {
            return Ok(());
        }
        let stop = Arc::new(AtomicBool::new(false));
        map.insert(session_id.to_string(), stop.clone());
        let sid = session_id.to_string();
        thread::Builder::new()
            .name(format!("transcript-{}", &sid[..sid.len().min(8)]))
            .spawn(move || tail_loop(app, sid, engine, stop))
            .map_err(|e| AppError::Other(format!("transcript watcher thread: {e}")))?;
        Ok(())
    }

    pub fn unwatch(&self, session_id: &str) {
        if let Some(stop) = self.inner.lock().remove(session_id) {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

fn locate(engine: &str, session_id: &str) -> Option<PathBuf> {
    match engine {
        "claude" => locate_transcript(session_id).ok(),
        "codex" => find_codex_rollout(session_id),
        _ => None,
    }
}

fn tail_loop(app: AppHandle, session_id: String, engine: String, stop: Arc<AtomicBool>) {
    let path = loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if let Some(p) = locate(&engine, &session_id) {
            break p;
        }
        thread::sleep(LOCATE_RETRY);
    };
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let (watcher, rx, interval) = dir_wake_source(&dir, Duration::from_millis(250));
    let mut last_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let items = backfill(&path, last_size, &engine);
    if !items.is_empty() {
        let _ = app.emit(
            "transcript://delta",
            TranscriptDelta {
                session_id: session_id.clone(),
                engine: engine.clone(),
                backfill: true,
                items,
            },
        );
    }
    loop {
        await_wake(&rx, interval, watcher.is_some());
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let (lines, next) = drain_new_lines(&path, last_size);
        last_size = next;
        let items: Vec<TranscriptItem> =
            lines.iter().flat_map(|v| parse_line(&engine, v)).collect();
        if items.is_empty() {
            continue;
        }
        let _ = app.emit(
            "transcript://delta",
            TranscriptDelta {
                session_id: session_id.clone(),
                engine: engine.clone(),
                backfill: false,
                items,
            },
        );
    }
}

/// The tail of an existing transcript since its last human prompt (bounded
/// by `BACKFILL_BYTES`), so a session opened mid-turn shows the turn so far.
fn backfill(path: &Path, size: u64, engine: &str) -> Vec<TranscriptItem> {
    let start = size.saturating_sub(BACKFILL_BYTES);
    let Ok(mut f) = fs::File::open(path) else {
        return Vec::new();
    };
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    // Skip the (possibly torn) first line when we did not start at 0.
    let body = if start > 0 {
        text.find('\n').map(|i| &text[i + 1..]).unwrap_or("")
    } else {
        &text
    };
    let items: Vec<TranscriptItem> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .flat_map(|v| parse_line(engine, &v))
        .collect();
    since_last_prompt(items)
}

/// Keep the last prompt and everything after it.
pub fn since_last_prompt(items: Vec<TranscriptItem>) -> Vec<TranscriptItem> {
    match items
        .iter()
        .rposition(|i| matches!(i, TranscriptItem::User { .. }))
    {
        Some(idx) => items[idx..].to_vec(),
        None => items,
    }
}

pub fn parse_line(engine: &str, v: &Value) -> Vec<TranscriptItem> {
    match engine {
        "claude" => parse_claude_line(v),
        "codex" => parse_codex_line(v),
        _ => Vec::new(),
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// A glance at a tool's input: the command, the path, the pattern — else the
/// first string value, else the compact JSON — cut to `INPUT_EXCERPT_CHARS`.
pub fn input_excerpt(input: &Value) -> String {
    let picked = [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "cmd",
    ]
    .iter()
    .find_map(|k| str_at(input, k))
    .or_else(|| {
        input
            .as_object()
            .and_then(|o| o.values().find_map(Value::as_str))
    })
    .map(str::to_string)
    .or_else(|| input.as_str().map(str::to_string))
    .unwrap_or_else(|| {
        if input.is_null() {
            String::new()
        } else {
            input.to_string()
        }
    });
    let one_line = picked.lines().next().unwrap_or("").trim().to_string();
    truncate_chars(&one_line, INPUT_EXCERPT_CHARS)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// One line of Claude's session transcript. `assistant` lines carry one
/// content block each (text, tool_use, thinking); `user` lines are either the
/// human's prompt (string content) or tool results (array content).
pub fn parse_claude_line(v: &Value) -> Vec<TranscriptItem> {
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return Vec::new();
    }
    let ts = str_at(v, "timestamp").and_then(iso_to_ms).unwrap_or(0);
    let uuid = str_at(v, "uuid").unwrap_or("");
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return Vec::new();
    };
    match str_at(v, "type") {
        Some("assistant") => content
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| match str_at(b, "type") {
                        Some("text") => {
                            let text = str_at(b, "text").unwrap_or("").trim();
                            (!text.is_empty()).then(|| TranscriptItem::Text {
                                id: format!("{uuid}:{i}"),
                                ts,
                                text: text.to_string(),
                            })
                        }
                        Some("tool_use") => Some(TranscriptItem::ToolUse {
                            id: str_at(b, "id").unwrap_or(uuid).to_string(),
                            ts,
                            name: str_at(b, "name").unwrap_or("tool").to_string(),
                            input: input_excerpt(b.get("input").unwrap_or(&Value::Null)),
                        }),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Some("user") => {
            if let Some(text) = content.as_str() {
                let text = text.trim();
                return if text.is_empty() {
                    Vec::new()
                } else {
                    vec![TranscriptItem::User {
                        id: uuid.to_string(),
                        ts,
                        text: text.to_string(),
                    }]
                };
            }
            let meta = v.get("isMeta").and_then(Value::as_bool) == Some(true);
            content
                .as_array()
                .map(|blocks| {
                    blocks
                        .iter()
                        .enumerate()
                        .filter_map(|(i, b)| match str_at(b, "type") {
                            Some("tool_result") => Some(TranscriptItem::ToolResult {
                                id: str_at(b, "tool_use_id").unwrap_or(uuid).to_string(),
                                ts,
                                ok: b.get("is_error").and_then(Value::as_bool) != Some(true),
                            }),
                            Some("text") if !meta => {
                                let text = str_at(b, "text").unwrap_or("").trim();
                                (!text.is_empty()).then(|| TranscriptItem::User {
                                    id: format!("{uuid}:{i}"),
                                    ts,
                                    text: text.to_string(),
                                })
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// One line of a Codex rollout: `response_item`s are the thread (messages,
/// tool calls and their outputs); `event_msg`s are telemetry we leave to the
/// usage reader. Reasoning is encrypted and skipped.
pub fn parse_codex_line(v: &Value) -> Vec<TranscriptItem> {
    if str_at(v, "type") != Some("response_item") {
        return Vec::new();
    }
    let ts = str_at(v, "timestamp").and_then(iso_to_ms).unwrap_or(0);
    let Some(p) = v.get("payload") else {
        return Vec::new();
    };
    let id = str_at(p, "id").unwrap_or("").to_string();
    let joined = |kind: &str| -> String {
        p.get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| str_at(b, "type") == Some(kind))
                    .filter_map(|b| str_at(b, "text"))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    match str_at(p, "type") {
        Some("message") => match str_at(p, "role") {
            Some("assistant") => {
                let text = joined("output_text");
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![TranscriptItem::Text { id, ts, text }]
                }
            }
            Some("user") => {
                let text = joined("input_text");
                // The harness's own context dumps are user-role too.
                if text.is_empty() || text.starts_with('<') {
                    Vec::new()
                } else {
                    vec![TranscriptItem::User { id, ts, text }]
                }
            }
            _ => Vec::new(),
        },
        Some("function_call") | Some("custom_tool_call") => {
            let args = p
                .get("arguments")
                .or_else(|| p.get("input"))
                .cloned()
                .unwrap_or(Value::Null);
            // Arguments arrive as a JSON string; look inside when they parse.
            let input = match args
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
            {
                Some(parsed) => input_excerpt(&parsed),
                None => input_excerpt(&args),
            };
            vec![TranscriptItem::ToolUse {
                id: str_at(p, "call_id").unwrap_or(&id).to_string(),
                ts,
                name: str_at(p, "name").unwrap_or("tool").to_string(),
                input,
            }]
        }
        Some("function_call_output") | Some("custom_tool_call_output") => {
            vec![TranscriptItem::ToolResult {
                id: str_at(p, "call_id").unwrap_or(&id).to_string(),
                ts,
                ok: true,
            }]
        }
        _ => Vec::new(),
    }
}

/// `2026-10-16T17:25:58.553Z` → epoch milliseconds. Only the UTC form the two
/// CLIs write; anything else is `None` and the item sorts by arrival.
pub fn iso_to_ms(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    let (hms, frac) = match time.split_once('.') {
        Some((a, b)) => (a, b),
        None => (time, ""),
    };
    let mut t = hms.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let min: i64 = t.next()?.parse().ok()?;
    let sec: i64 = t.next()?.parse().ok()?;
    let millis: i64 = if frac.is_empty() {
        0
    } else {
        let digits: String = frac.chars().take(3).collect();
        let padded = format!("{digits:0<3}");
        padded.parse().ok()?
    };
    // Days from civil (Howard Hinnant), valid for the proleptic Gregorian calendar.
    let (y, m) = if m <= 2 {
        (y - 1, m as i64 + 9)
    } else {
        (y, m as i64 - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((((days * 24 + h) * 60 + min) * 60 + sec) * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn iso_timestamps_become_epoch_ms() {
        assert_eq!(iso_to_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_to_ms("1970-01-02T00:00:00.5Z"), Some(86_400_500));
        assert_eq!(
            iso_to_ms("2026-10-16T17:25:58.553Z"),
            Some(1_792_171_558_553)
        );
        assert_eq!(iso_to_ms("2026-10-16 17:25:58"), None);
    }

    #[test]
    fn claude_assistant_blocks_become_text_and_tool_use_and_thinking_is_skipped() {
        let line = json!({
            "type": "assistant", "uuid": "a1", "timestamp": "2026-10-16T17:25:58.553Z",
            "message": {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "hmm"},
                {"type": "text", "text": "  Running the tests now.  "},
                {"type": "tool_use", "id": "toolu_1", "name": "Bash",
                 "input": {"command": "npm test -- --run\n--verbose", "description": "Run tests"}}
            ]}
        });
        assert_eq!(
            parse_claude_line(&line),
            vec![
                TranscriptItem::Text {
                    id: "a1:1".into(),
                    ts: 1_792_171_558_553,
                    text: "Running the tests now.".into()
                },
                TranscriptItem::ToolUse {
                    id: "toolu_1".into(),
                    ts: 1_792_171_558_553,
                    name: "Bash".into(),
                    input: "npm test -- --run".into()
                },
            ]
        );
    }

    #[test]
    fn claude_user_lines_are_prompts_or_tool_results_and_sidechains_are_dropped() {
        let prompt = json!({"type":"user","uuid":"u1","timestamp":"2026-10-16T17:25:50Z",
            "message":{"role":"user","content":"fix the build"}});
        assert_eq!(
            parse_claude_line(&prompt),
            vec![TranscriptItem::User {
                id: "u1".into(),
                ts: 1_792_171_550_000,
                text: "fix the build".into()
            }]
        );
        let result = json!({"type":"user","uuid":"u2","timestamp":"2026-10-16T17:25:59Z",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"boom","is_error":true}]}});
        assert_eq!(
            parse_claude_line(&result),
            vec![TranscriptItem::ToolResult {
                id: "toolu_1".into(),
                ts: 1_792_171_559_000,
                ok: false
            }]
        );
        let meta = json!({"type":"user","uuid":"u3","isMeta":true,"timestamp":"2026-10-16T17:25:59Z",
            "message":{"role":"user","content":[{"type":"text","text":"Base directory for this skill: /x"}]}});
        assert!(parse_claude_line(&meta).is_empty());
        let side = json!({"type":"assistant","uuid":"a9","isSidechain":true,"timestamp":"2026-10-16T17:25:59Z",
            "message":{"role":"assistant","content":[{"type":"text","text":"subagent says"}]}});
        assert!(parse_claude_line(&side).is_empty());
        let housekeeping = json!({"type":"queue-operation","operation":"enqueue"});
        assert!(parse_claude_line(&housekeeping).is_empty());
    }

    #[test]
    fn codex_response_items_become_text_tool_calls_and_results() {
        let msg = json!({"timestamp":"2026-10-16T17:25:58.553Z","type":"response_item",
            "payload":{"type":"message","id":"msg_1","role":"assistant",
                "content":[{"type":"output_text","text":"Voy a revisar"},{"type":"output_text","text":"el estado."}]}});
        assert_eq!(
            parse_codex_line(&msg),
            vec![TranscriptItem::Text {
                id: "msg_1".into(),
                ts: 1_792_171_558_553,
                text: "Voy a revisar\n\nel estado.".into()
            }]
        );
        let call = json!({"timestamp":"2026-10-16T17:25:58.883Z","type":"response_item",
            "payload":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"shell",
                "arguments":"{\"command\":[\"git\",\"status\"]}"}});
        let got = parse_codex_line(&call);
        assert!(
            matches!(&got[0], TranscriptItem::ToolUse { id, name, .. } if id == "call_1" && name == "shell")
        );
        let custom = json!({"timestamp":"2026-10-16T17:25:58.883Z","type":"response_item",
            "payload":{"type":"custom_tool_call","id":"ctc_1","call_id":"call_2","name":"exec","input":"ls -la"}});
        assert!(
            matches!(&parse_codex_line(&custom)[0], TranscriptItem::ToolUse { input, .. } if input == "ls -la")
        );
        let out = json!({"timestamp":"2026-10-16T17:26:00.582Z","type":"response_item",
            "payload":{"type":"custom_tool_call_output","id":"ctco_1","call_id":"call_2","output":"ok"}});
        assert_eq!(
            parse_codex_line(&out),
            vec![TranscriptItem::ToolResult {
                id: "call_2".into(),
                ts: 1_792_171_560_582,
                ok: true
            }]
        );
        let user = json!({"timestamp":"2026-10-16T17:25:54.682Z","type":"response_item",
            "payload":{"type":"message","id":"msg_u","role":"user","content":[{"type":"input_text","text":"<environment_context>…"}]}});
        assert!(parse_codex_line(&user).is_empty());
        let reasoning =
            json!({"type":"response_item","payload":{"type":"reasoning","encrypted_content":"x"}});
        assert!(parse_codex_line(&reasoning).is_empty());
        let event = json!({"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"done"}});
        assert!(parse_codex_line(&event).is_empty());
    }

    #[test]
    fn excerpt_prefers_the_command_then_a_path_and_cuts_long_input() {
        assert_eq!(
            input_excerpt(&json!({"description":"x","command":"cargo test"})),
            "cargo test"
        );
        assert_eq!(
            input_excerpt(&json!({"file_path":"/a/b.rs","content":"..."})),
            "/a/b.rs"
        );
        assert_eq!(
            input_excerpt(&json!({"skill":"deep-research"})),
            "deep-research"
        );
        let long = "x".repeat(300);
        assert_eq!(
            input_excerpt(&json!({"command": long})).chars().count(),
            INPUT_EXCERPT_CHARS
        );
        assert_eq!(input_excerpt(&Value::Null), "");
    }

    #[test]
    fn backfill_keeps_the_last_prompt_onwards() {
        let t = |id: &str| TranscriptItem::Text {
            id: id.into(),
            ts: 1,
            text: "t".into(),
        };
        let u = |id: &str| TranscriptItem::User {
            id: id.into(),
            ts: 1,
            text: "u".into(),
        };
        let got = since_last_prompt(vec![t("a"), u("p1"), t("b"), u("p2"), t("c")]);
        assert_eq!(got, vec![u("p2"), t("c")]);
        assert_eq!(since_last_prompt(vec![t("a")]), vec![t("a")]);
    }
}
