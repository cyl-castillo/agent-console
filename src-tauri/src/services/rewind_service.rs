//! Rewind a Claude conversation by forking its transcript, truncated at a
//! turn boundary (M9).
//!
//! There is no official way to rewind a conversation from outside the CLI:
//! `--fork-session` only forks from the END, the SDK's file checkpointing
//! explicitly does not rewind the conversation, and the interactive `/rewind`
//! mutates the session in place. What DOES work (verified live on claude
//! 2.1.248, 2026-09-02): copy `~/.claude/projects/<slug>/<sid>.jsonl`
//! truncated right before the first line of the turn AFTER the one we rewind
//! to, save it as `<new-uuid>.jsonl` in the SAME directory, and `claude
//! --resume <new-uuid>` starts with the memory rewound. Non-destructive: the
//! original transcript is never opened for writing.
//!
//! What "first line of the next turn" is depends on how the session runs —
//! headless transcripts mark turns with `queue-operation enqueue` lines,
//! interactive ones (the only kind the GUI produces) with `user` lines
//! carrying a fresh `promptId`. `find_boundary` documents and enforces both
//! grammars.
//!
//! This is a conscious workaround over an internal format that churns between
//! releases, so every step is defensive and fails LOUDLY into an honest
//! degradation ("files restored, agent memory NOT rewound") instead of
//! forking garbage:
//! - the CLI version is gated on `claude --version` (not on hook markers);
//! - the transcript is located by scanning `~/.claude/projects/*/` for the
//!   session file — the slug encoding changed in 2.1.224, so re-deriving it
//!   from the cwd would silently miss transcripts;
//! - every line before the cut must parse as JSON, every enqueue line must
//!   carry our session id and a parseable timestamp, and a transcript with no
//!   enqueue lines at all is treated as a format change, not as "no turns".

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::error::{AppError, AppResult};

/// Oldest CLI version the fork is attempted on. The workaround was verified
/// live on 2.1.248; 2.1.224 is the conservative floor (same transcript-format
/// era — it's the release that settled the current projects-dir encoding).
/// There is no ceiling: a future format change is caught by the parse-level
/// checks below, which refuse to fork rather than fork wrong.
pub const MIN_CLAUDE_FORK_VERSION: (u64, u64, u64) = (2, 1, 224);

/// Same contract as `isSafeSessionId` on the TS side (profiles.ts): the id
/// names a file we create and lands in a `--resume` command line, and it
/// originates from world-writable state — validate at the boundary.
pub fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub struct ForkOutcome {
    /// The uuid of the new transcript — what `claude --resume` takes.
    pub fork_session_id: String,
    /// False when the rewound-to turn was already the last one (the fork is a
    /// byte-identical copy, which still gives resume a non-destructive base).
    pub truncated: bool,
}

/// Locate `<session_id>.jsonl` by scanning every slug directory under
/// `~/.claude/projects/`. Deliberately NOT derived from the cwd: the slug
/// encoding changed in claude 2.1.224 and may change again — the session id
/// is a uuid, so a filename match is unambiguous. Newest mtime wins in the
/// (theoretical) case of duplicates.
pub fn locate_transcript(session_id: &str) -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    let projects = home.join(".claude").join("projects");
    let file_name = format!("{session_id}.jsonl");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(&projects).map_err(|_| {
        AppError::NotFound(format!(
            "claude projects dir not found: {}",
            projects.display()
        ))
    })? {
        let Ok(entry) = entry else { continue };
        let candidate = entry.path().join(&file_name);
        let Ok(meta) = candidate.metadata() else {
            continue;
        };
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(t, _)| mtime > *t) {
            best = Some((mtime, candidate));
        }
    }
    best.map(|(_, p)| p).ok_or_else(|| {
        AppError::NotFound(format!(
            "no transcript {file_name} under {}",
            projects.display()
        ))
    })
}

/// Fork the transcript of `session_id`, keeping everything up to and
/// including the turn whose Stop fired at `cutoff_ms` (epoch ms, from the
/// ledger's turn_end event). The cut lands right before the first enqueue
/// line strictly AFTER the cutoff — the boundary the live experiment proved.
pub fn fork_transcript(session_id: &str, cutoff_ms: i64) -> AppResult<ForkOutcome> {
    if !is_safe_session_id(session_id) {
        return Err(AppError::InvalidArgument("unsafe session id".into()));
    }
    let path = locate_transcript(session_id)?;
    let data = fs::read_to_string(&path)?;
    let cut = find_boundary(&data, session_id, cutoff_ms)?;

    let fork_id = uuid::Uuid::new_v4().to_string();
    let dir = path
        .parent()
        .ok_or_else(|| AppError::Other("transcript has no parent dir".into()))?;
    // Write-then-rename: resume must never see a half-written transcript.
    let tmp = dir.join(format!(".{fork_id}.jsonl.tmp"));
    fs::write(&tmp, &data[..cut])?;
    fs::rename(&tmp, dir.join(format!("{fork_id}.jsonl")))?;
    Ok(ForkOutcome {
        fork_session_id: fork_id,
        truncated: cut < data.len(),
    })
}

/// Byte offset to truncate `data` at. Errors are format-drift tripwires: this
/// parser must BREAK, not guess, when a claude release changes the transcript.
///
/// Two turn-start grammars coexist (a session can even mix them — started
/// headless, resumed interactive):
///
/// - **Headless** (`claude -p`): every prompt is preceded by a
///   `{"type":"queue-operation","operation":"enqueue",...}` line. That line is
///   the boundary (the shape the original live experiment verified).
/// - **Interactive** (the only kind the GUI produces): there are NO
///   queue-operation lines. A turn starts at a non-sidechain `user` line
///   carrying a `promptId` never seen before in the file — tool results also
///   arrive as `user` lines, but they REUSE their turn's promptId, so novelty
///   is what distinguishes a prompt from a result. Verified against a real
///   2.1.248 interactive transcript. The per-turn header block
///   (`mode`/`permission-mode`/`atis-latch`/`bridge-session`) is deliberately
///   NOT a boundary: it re-appears mid-turn (e.g. around a permission
///   approval), so cutting there would split a turn in half.
///
/// The scan is strictly in file order and the first marker past the cutoff
/// wins — in headless files the enqueue precedes its `user` line, so the cut
/// stays at the enqueue exactly as before.
fn find_boundary(data: &str, session_id: &str, cutoff_ms: i64) -> AppResult<usize> {
    let mut offset = 0usize;
    let mut saw_turn_marker = false;
    let mut seen_prompt_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut line_no = 0usize;
    while offset < data.len() {
        line_no += 1;
        let rest = &data[offset..];
        let (line, advance) = match rest.find('\n') {
            Some(i) => (&rest[..i], i + 1),
            None => (rest, rest.len()),
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            offset += advance;
            continue;
        }
        let parsed: Result<Value, _> = serde_json::from_str(trimmed);
        let Ok(v) = parsed else {
            if offset + advance >= data.len() {
                // A torn final line is expected on a live session (the writer
                // may be mid-append). It can only be dropped, never kept.
                break;
            }
            return Err(AppError::Other(format!(
                "transcript line {line_no} is not valid JSON — format may have changed"
            )));
        };
        let line_type = v.get("type").and_then(Value::as_str);
        if line_type == Some("queue-operation")
            && v.get("operation").and_then(Value::as_str) == Some("enqueue")
        {
            saw_turn_marker = true;
            let sid = v.get("sessionId").and_then(Value::as_str).ok_or_else(|| {
                AppError::Other(format!(
                    "enqueue line {line_no} has no sessionId — format may have changed"
                ))
            })?;
            if sid != session_id {
                return Err(AppError::Other(format!(
                    "enqueue line {line_no} belongs to session {sid}, expected {session_id}"
                )));
            }
            let ms = line_timestamp_ms(&v, line_no, "enqueue")?;
            if ms > cutoff_ms {
                return Ok(offset);
            }
        } else if line_type == Some("user")
            && !v
                .get("isSidechain")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            // Interactive grammar. Sidechain (subagent) lines never open a
            // top-level turn, hence the guard above.
            if let Some(pid) = v.get("promptId").and_then(Value::as_str) {
                saw_turn_marker = true;
                if seen_prompt_ids.insert(pid.to_string()) {
                    // First time this promptId appears = a turn starts HERE.
                    let ms = line_timestamp_ms(&v, line_no, "user prompt")?;
                    if ms > cutoff_ms {
                        return Ok(offset);
                    }
                }
            }
        }
        offset += advance;
    }
    if !saw_turn_marker {
        return Err(AppError::Other(
            "no turn markers in transcript (neither queue-operation enqueue nor promptId user \
             lines) — format may have changed"
                .into(),
        ));
    }
    // Every prompt happened at or before the cutoff: the rewound-to turn is
    // the last one, so the fork keeps every fully-written line.
    Ok(offset)
}

/// The line's `timestamp` field as epoch ms — required on any line the scan
/// evaluates as a turn boundary. Missing/unparseable = drift, break loudly.
fn line_timestamp_ms(v: &Value, line_no: usize, what: &str) -> AppResult<i64> {
    let ts = v.get("timestamp").and_then(Value::as_str).ok_or_else(|| {
        AppError::Other(format!(
            "{what} line {line_no} has no timestamp — format may have changed"
        ))
    })?;
    iso_to_epoch_ms(ts).ok_or_else(|| {
        AppError::Other(format!(
            "{what} line {line_no} timestamp {ts:?} is not ISO 8601 UTC"
        ))
    })
}

/// `"2026-09-02T12:09:30.203Z"` → epoch ms. Only the exact shape claude
/// writes (UTC, `Z` suffix, optional fractional seconds) — anything else is
/// format drift and returns None.
fn iso_to_epoch_ms(ts: &str) -> Option<i64> {
    let ts = ts.strip_suffix('Z')?;
    let (date, time) = ts.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    if d.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (hms, frac) = match time.split_once('.') {
        Some((h, f)) => (h, f),
        None => (time, ""),
    };
    let mut t = hms.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let min: i64 = t.next()?.parse().ok()?;
    let sec: i64 = t.next()?.parse().ok()?;
    if t.next().is_some() || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let millis: i64 = if frac.is_empty() {
        0
    } else {
        // Truncate/pad to milliseconds; the fraction must be all digits.
        if !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let padded = format!("{frac:0<3}");
        padded[..3].parse().ok()?
    };
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + min) * 60_000 + sec * 1000 + millis)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Gate the fork on `claude --version`. Err carries the honest reason the UI
/// shows next to "agent memory NOT rewound". Deliberately NOT gated on hook
/// markers: the trust marker is known-broken (W5), and `--version` answers
/// for the binary that will actually run `--resume`.
pub fn fork_gate() -> Result<(), String> {
    let line = version_line().ok_or_else(|| "could not run `claude --version`".to_string())?;
    let v = parse_semver(&line)
        .ok_or_else(|| format!("unrecognized `claude --version` output: {line}"))?;
    check_min_version(v)
}

fn check_min_version(v: (u64, u64, u64)) -> Result<(), String> {
    let (a, b, c) = MIN_CLAUDE_FORK_VERSION;
    if v < MIN_CLAUDE_FORK_VERSION {
        return Err(format!(
            "claude {}.{}.{} predates {a}.{b}.{c}, the oldest release the transcript fork is verified on",
            v.0, v.1, v.2
        ));
    }
    Ok(())
}

/// First `MAJOR.MINOR.PATCH` in the line (claude prints e.g.
/// `2.1.248 (Claude Code)`).
fn parse_semver(line: &str) -> Option<(u64, u64, u64)> {
    let re = regex::Regex::new(r"(\d+)\.(\d+)\.(\d+)").ok()?;
    let c = re.captures(line)?;
    Some((c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?))
}

/// First line of `claude --version`, through a login shell so the PATH is the
/// user's real one (same pattern as the preflight probe).
#[cfg(not(windows))]
fn version_line() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let script = "command -v claude >/dev/null 2>&1 && claude --version 2>/dev/null | head -n1";
    let out = crate::services::proc::command(&shell)
        .args(["-lc", script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!line.is_empty()).then_some(line)
}

#[cfg(windows)]
fn version_line() -> Option<String> {
    let out = crate::services::proc::command("claude")
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!line.is_empty()).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    fn enqueue(ts: &str, content: &str) -> String {
        format!(
            r#"{{"type":"queue-operation","operation":"enqueue","timestamp":"{ts}","sessionId":"{SID}","content":"{content}"}}"#
        )
    }

    fn dequeue(ts: &str) -> String {
        format!(
            r#"{{"type":"queue-operation","operation":"dequeue","timestamp":"{ts}","sessionId":"{SID}"}}"#
        )
    }

    fn user_prompt(ts: &str, prompt_id: &str, content: &str) -> String {
        format!(
            r#"{{"parentUuid":null,"isSidechain":false,"promptId":"{prompt_id}","type":"user","message":{{"role":"user","content":"{content}"}},"uuid":"u-{prompt_id}","timestamp":"{ts}"}}"#
        )
    }

    /// A tool result ALSO arrives as a `user` line, reusing its turn's
    /// promptId — the trap that makes "user line = new turn" a wrong parser.
    fn tool_result_user(ts: &str, prompt_id: &str, tool_use_id: &str) -> String {
        format!(
            r#"{{"parentUuid":"u-{prompt_id}","isSidechain":false,"promptId":"{prompt_id}","type":"user","message":{{"role":"user","content":[{{"tool_use_id":"{tool_use_id}","type":"tool_result","content":"ok"}}]}},"uuid":"u-{tool_use_id}","timestamp":"{ts}"}}"#
        )
    }

    fn assistant(ts: &str, text: &str) -> String {
        format!(
            r#"{{"parentUuid":"x","isSidechain":false,"message":{{"model":"claude-opus-5","id":"msg_x","type":"message","role":"assistant","content":[{{"type":"text","text":"{text}"}}]}},"type":"assistant","uuid":"a-{ts}","timestamp":"{ts}"}}"#
        )
    }

    /// The per-turn header block of an interactive session. It ALSO re-appears
    /// mid-turn (e.g. around a permission approval), which is why it must
    /// never be treated as a turn boundary.
    fn header_block() -> [String; 4] {
        [
            format!(r#"{{"type":"mode","mode":"normal","sessionId":"{SID}"}}"#),
            format!(r#"{{"type":"permission-mode","permissionMode":"auto","sessionId":"{SID}"}}"#),
            format!(r#"{{"type":"atis-latch","atis":"","sessionId":"{SID}"}}"#),
            format!(
                r#"{{"type":"bridge-session","sessionId":"{SID}","bridgeSessionId":"cse_01X","lastSequenceNum":0}}"#
            ),
        ]
    }

    /// Minimal synthetic HEADLESS (`claude -p`) transcript with three turns.
    /// This test doubles as the format smoke test: it encodes the exact line
    /// shapes the fork relies on, and it must be updated deliberately if a
    /// claude release changes them. The `user` lines carry promptId/timestamp
    /// like the real ones — proving the enqueue line (which comes first) stays
    /// the boundary in headless files.
    fn synthetic() -> String {
        [
            enqueue("2026-09-02T10:00:00.000Z", "turn one"),
            dequeue("2026-09-02T10:00:00.100Z"),
            user_prompt("2026-09-02T10:00:00.200Z", "hp-one", "turn one"),
            assistant("2026-09-02T10:00:05.000Z", "done one"),
            enqueue("2026-09-02T11:00:00.000Z", "turn two"),
            dequeue("2026-09-02T11:00:00.100Z"),
            user_prompt("2026-09-02T11:00:00.200Z", "hp-two", "turn two"),
            enqueue("2026-09-02T12:00:00.000Z", "turn three"),
            dequeue("2026-09-02T12:00:00.100Z"),
            user_prompt("2026-09-02T12:00:00.200Z", "hp-three", "turn three"),
        ]
        .join("\n")
            + "\n"
    }

    /// Minimal synthetic INTERACTIVE transcript, line shapes cloned from a
    /// real claude 2.1.248 session (~/.claude/projects/…/09bdeb80….jsonl, the
    /// GUI-verification exemplar): per-turn header block, file-history
    /// snapshot, prompt as a `user` line with a fresh promptId, tool results
    /// as `user` lines REUSING that promptId, a mid-turn header re-latch
    /// (around an approval), system stop/turn_duration closers, and
    /// last-prompt/ai-title/cost-state trailers. Same contract as the
    /// headless fixture: a claude release changing these shapes must turn CI
    /// red, deliberately.
    fn synthetic_interactive() -> String {
        let h = header_block();
        [
            // ---- turn one ----
            h[0].clone(),
            h[1].clone(),
            h[2].clone(),
            h[3].clone(),
            format!(
                r#"{{"type":"file-history-snapshot","messageId":"m1","snapshot":{{"messageId":"m1","trackedFileBackups":{{}},"timestamp":"2026-09-03T01:00:04.760Z"}},"isSnapshotUpdate":false}}"#
            ),
            user_prompt("2026-09-03T01:00:04.694Z", "prompt-one", "turn one codeword ROJO"),
            format!(
                r#"{{"parentUuid":"u-prompt-one","isSidechain":false,"attachment":{{"type":"total_tokens_reminder","text":"x"}},"type":"attachment","uuid":"att1","timestamp":"2026-09-03T01:00:04.800Z"}}"#
            ),
            format!(r#"{{"type":"ai-title","aiTitle":"demo","sessionId":"{SID}"}}"#),
            assistant("2026-09-03T01:00:10.000Z", "working on it"),
            format!(
                r#"{{"parentUuid":"a1","isSidechain":false,"attachment":{{"type":"hook_success","hookName":"PreToolUse:Write","toolUseID":"toolu_1","hookEvent":"PreToolUse","content":""}},"type":"attachment","uuid":"att2","timestamp":"2026-09-03T01:00:39.300Z"}}"#
            ),
            tool_result_user("2026-09-03T01:00:39.343Z", "prompt-one", "toolu_1"),
            // Mid-turn header re-latch (the approval pause) — NOT a boundary.
            h[0].clone(),
            h[1].clone(),
            h[2].clone(),
            h[3].clone(),
            assistant("2026-09-03T01:01:10.000Z", "Done, ROJO set"),
            format!(
                r#"{{"parentUuid":"a2","isSidechain":false,"type":"system","subtype":"stop_hook_summary","hookCount":1,"hookInfos":[],"timestamp":"2026-09-03T01:01:12.700Z","uuid":"sys1","isMeta":false}}"#
            ),
            format!(
                r#"{{"parentUuid":"sys1","isSidechain":false,"type":"system","subtype":"turn_duration","durationMs":67987,"messageCount":18,"timestamp":"2026-09-03T01:01:12.750Z","uuid":"sys2","isMeta":false}}"#
            ),
            format!(
                r#"{{"type":"file-history-delta","messageId":"m2","snapshotMessageId":"m1","trackingPath":"version.txt"}}"#
            ),
            format!(
                r#"{{"type":"last-prompt","lastPrompt":"turn one codeword ROJO","leafUuid":"sys2","sessionId":"{SID}"}}"#
            ),
            format!(r#"{{"type":"custom-title","customTitle":"demo","sessionId":"{SID}"}}"#),
            format!(r#"{{"type":"agent-name","agentName":"demo","sessionId":"{SID}"}}"#),
            // A sidechain (subagent) user line with a NOVEL promptId — never a
            // top-level turn boundary.
            format!(
                r#"{{"parentUuid":"sys2","isSidechain":true,"promptId":"sidechain-prompt","type":"user","message":{{"role":"user","content":"subagent task"}},"uuid":"sc1","timestamp":"2026-09-03T01:01:20.000Z"}}"#
            ),
            // ---- turn two ----
            user_prompt("2026-09-03T01:01:38.904Z", "prompt-two", "turn two codeword AZUL"),
            format!(
                r#"{{"type":"file-history-snapshot","messageId":"m3","snapshot":{{"messageId":"m3","trackedFileBackups":{{}},"timestamp":"2026-09-03T01:01:39.000Z"}},"isSnapshotUpdate":false}}"#
            ),
            assistant("2026-09-03T01:01:45.000Z", "Done, AZUL set"),
            format!(
                r#"{{"parentUuid":"a3","isSidechain":false,"type":"system","subtype":"turn_duration","durationMs":36470,"messageCount":27,"timestamp":"2026-09-03T01:02:15.417Z","uuid":"sys3","isMeta":false}}"#
            ),
            format!(
                r#"{{"type":"cost-state","sessionId":"{SID}","totalCostUSD":1.1,"totalAPIDuration":22019}}"#
            ),
            format!(
                r#"{{"type":"last-prompt","lastPrompt":"turn two codeword AZUL","leafUuid":"sys3","sessionId":"{SID}"}}"#
            ),
        ]
        .join("\n")
            + "\n"
    }

    /// 2026-09-02T10:30Z as epoch ms — a Stop between turn one and turn two.
    const AFTER_TURN_ONE_MS: i64 = 1_788_345_000_000;

    #[test]
    fn iso_parsing_matches_known_epochs() {
        assert_eq!(iso_to_epoch_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            iso_to_epoch_ms("2026-09-02T12:09:30.203Z"),
            Some(1_788_350_970_203)
        );
        // Fraction shorter/longer than millis still lands on millis.
        assert_eq!(iso_to_epoch_ms("1970-01-01T00:00:00.5Z"), Some(500));
        assert_eq!(iso_to_epoch_ms("1970-01-01T00:00:00.123456Z"), Some(123));
        // Drifted shapes must fail, not guess.
        assert_eq!(iso_to_epoch_ms("2026-09-02 12:09:30Z"), None);
        assert_eq!(iso_to_epoch_ms("2026-09-02T12:09:30+00:00"), None);
        assert_eq!(iso_to_epoch_ms("not a date"), None);
    }

    #[test]
    fn boundary_cuts_before_next_enqueue_after_cutoff() {
        let data = synthetic();
        let cut = find_boundary(&data, SID, AFTER_TURN_ONE_MS).unwrap();
        let kept = &data[..cut];
        assert!(kept.contains("turn one"));
        assert!(!kept.contains("turn two"));
        assert!(!kept.contains("turn three"));
        // The cut lands exactly at a line start.
        assert!(data[cut..].starts_with(r#"{"type":"queue-operation","operation":"enqueue""#));
    }

    #[test]
    fn boundary_after_last_turn_keeps_everything() {
        let data = synthetic();
        let cut = find_boundary(&data, SID, i64::MAX).unwrap();
        assert_eq!(cut, data.len());
    }

    #[test]
    fn torn_final_line_is_dropped_not_fatal() {
        let mut data = synthetic();
        data.push_str(r#"{"type":"queue-op"#); // writer caught mid-append
        let cut = find_boundary(&data, SID, i64::MAX).unwrap();
        assert_eq!(cut, synthetic().len());
    }

    #[test]
    fn interactive_boundary_cuts_before_the_next_prompt() {
        let data = synthetic_interactive();
        // The ledger's turn_end for turn one lands just after its
        // turn_duration line (01:01:12.750).
        let cutoff = iso_to_epoch_ms("2026-09-03T01:01:13Z").unwrap();
        let cut = find_boundary(&data, SID, cutoff).unwrap();
        let kept = &data[..cut];
        // Turn one survives WHOLE: closing message, system closers, trailers.
        assert!(kept.contains("Done, ROJO set"));
        assert!(kept.contains("turn_duration"));
        assert!(kept.contains("last-prompt"));
        assert!(kept.contains("custom-title"));
        // The sidechain line sits between the turns with a novel promptId and
        // a timestamp past the cutoff — if the parser treated it as a turn
        // start, the cut would land on it instead of on turn two's prompt.
        assert!(kept.contains("sidechain-prompt"));
        // Turn two is gone entirely...
        assert!(!kept.contains("AZUL"));
        assert!(!kept.contains("prompt-two"));
        // ...and the cut lands exactly on its prompt line.
        assert!(data[cut..]
            .starts_with(r#"{"parentUuid":null,"isSidechain":false,"promptId":"prompt-two""#));
    }

    #[test]
    fn interactive_tool_results_and_mid_turn_headers_are_not_boundaries() {
        // Cutoff mid-turn-one, BEFORE its tool result and the header
        // re-latch: a parser that treats any `user` line — or the header
        // block, which re-appears around approvals — as a turn start would
        // cut turn one in half right here.
        let data = synthetic_interactive();
        let cutoff = iso_to_epoch_ms("2026-09-03T01:00:10Z").unwrap();
        let cut = find_boundary(&data, SID, cutoff).unwrap();
        let kept = &data[..cut];
        assert!(kept.contains("toolu_1")); // the tool_result user line, kept
        assert!(kept.contains("Done, ROJO set")); // the closing message, kept
        assert!(!kept.contains("prompt-two"));
        assert!(data[cut..].contains("prompt-two"));
    }

    #[test]
    fn interactive_rewind_to_last_turn_keeps_everything() {
        let data = synthetic_interactive();
        assert_eq!(find_boundary(&data, SID, i64::MAX).unwrap(), data.len());
    }

    #[test]
    fn interactive_format_drift_breaks_loudly() {
        // A novel-promptId user line without a timestamp can't be placed
        // against the cutoff — drift, never a guess.
        let data = format!(
            r#"{{"type":"user","isSidechain":false,"promptId":"p1","message":{{"role":"user","content":"hi"}}}}"#
        ) + "\n";
        assert!(find_boundary(&data, SID, 0).is_err());
    }

    #[test]
    fn format_drift_breaks_loudly() {
        // Non-JSON in the middle of the file.
        let data = format!("not json at all\n{}", synthetic());
        assert!(find_boundary(&data, SID, 0).is_err());
        // No enqueue lines at all — a format change, not an empty session.
        let data = r#"{"type":"user","message":{}}"#.to_string() + "\n";
        assert!(find_boundary(&data, SID, 0).is_err());
        // Enqueue line without a timestamp.
        let data =
            format!(r#"{{"type":"queue-operation","operation":"enqueue","sessionId":"{SID}"}}"#)
                + "\n";
        assert!(find_boundary(&data, SID, 0).is_err());
    }

    #[test]
    fn foreign_session_id_refuses_to_fork() {
        let data = synthetic();
        assert!(find_boundary(&data, "11111111-2222-3333-4444-555555555555", 0).is_err());
    }

    #[test]
    fn session_id_validation_guards_the_filename() {
        assert!(is_safe_session_id(SID));
        assert!(!is_safe_session_id(""));
        assert!(!is_safe_session_id("../../etc/passwd"));
        assert!(!is_safe_session_id("a b"));
        assert!(!is_safe_session_id(&"a".repeat(129)));
    }

    #[test]
    fn version_gate_floor_and_parse() {
        assert!(parse_semver("2.1.248 (Claude Code)").is_some());
        assert_eq!(parse_semver("2.1.248 (Claude Code)"), Some((2, 1, 248)));
        assert_eq!(parse_semver("no digits here"), None);
        assert!(check_min_version((2, 1, 248)).is_ok());
        assert!(check_min_version((2, 1, 224)).is_ok());
        assert!(check_min_version((3, 0, 0)).is_ok());
        assert!(check_min_version((2, 1, 223)).is_err());
        assert!(check_min_version((1, 0, 999)).is_err());
    }

    #[test]
    fn fork_locates_scans_and_never_touches_the_original() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fake_home =
            std::env::temp_dir().join(format!("ac-rewind-home-{}-{nanos}", std::process::id()));
        // A slug the cwd-derived scheme would NEVER produce: the locator must
        // find the file by scanning, not by re-deriving the slug.
        let slug_dir = fake_home
            .join(".claude")
            .join("projects")
            .join("x--weird--encoding");
        fs::create_dir_all(&slug_dir).unwrap();
        let original = slug_dir.join(format!("{SID}.jsonl"));
        fs::write(&original, synthetic()).unwrap();

        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);
        let run = || -> AppResult<()> {
            assert_eq!(locate_transcript(SID)?, original);
            assert!(locate_transcript("00000000-0000-0000-0000-000000000000").is_err());

            let out = fork_transcript(SID, AFTER_TURN_ONE_MS)?;
            assert!(out.truncated);
            assert!(is_safe_session_id(&out.fork_session_id));
            assert_ne!(out.fork_session_id, SID);

            // Original byte-identical; fork truncated at the turn boundary.
            assert_eq!(fs::read_to_string(&original).unwrap(), synthetic());
            let fork_path = slug_dir.join(format!("{}.jsonl", out.fork_session_id));
            let fork = fs::read_to_string(&fork_path).unwrap();
            assert!(fork.contains("turn one"));
            assert!(!fork.contains("turn two"));
            assert!(fork.ends_with('\n'));

            // Rewind to the last turn → full copy, still a distinct file.
            let full = fork_transcript(SID, i64::MAX)?;
            assert!(!full.truncated);
            let copy = fs::read_to_string(slug_dir.join(format!("{}.jsonl", full.fork_session_id)))
                .unwrap();
            assert_eq!(copy, synthetic());
            Ok(())
        };
        let result = run();
        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&fake_home);
        result.unwrap();
    }
}
