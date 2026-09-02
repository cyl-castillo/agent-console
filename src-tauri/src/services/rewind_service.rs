//! Rewind a Claude conversation by forking its transcript, truncated at a
//! turn boundary (M9).
//!
//! There is no official way to rewind a conversation from outside the CLI:
//! `--fork-session` only forks from the END, the SDK's file checkpointing
//! explicitly does not rewind the conversation, and the interactive `/rewind`
//! mutates the session in place. What DOES work (verified live on claude
//! 2.1.248, 2026-09-02): copy `~/.claude/projects/<slug>/<sid>.jsonl`
//! truncated right before the `{"type":"queue-operation","operation":
//! "enqueue",...}` line of the first prompt AFTER the turn we rewind to, save
//! it as `<new-uuid>.jsonl` in the SAME directory, and `claude --resume
//! <new-uuid>` starts with the memory rewound. Non-destructive: the original
//! transcript is never opened for writing.
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
fn find_boundary(data: &str, session_id: &str, cutoff_ms: i64) -> AppResult<usize> {
    let mut offset = 0usize;
    let mut saw_enqueue = false;
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
        if v.get("type").and_then(Value::as_str) == Some("queue-operation")
            && v.get("operation").and_then(Value::as_str) == Some("enqueue")
        {
            saw_enqueue = true;
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
            let ts = v.get("timestamp").and_then(Value::as_str).ok_or_else(|| {
                AppError::Other(format!(
                    "enqueue line {line_no} has no timestamp — format may have changed"
                ))
            })?;
            let ms = iso_to_epoch_ms(ts).ok_or_else(|| {
                AppError::Other(format!(
                    "enqueue line {line_no} timestamp {ts:?} is not ISO 8601 UTC"
                ))
            })?;
            if ms > cutoff_ms {
                return Ok(offset);
            }
        }
        offset += advance;
    }
    if !saw_enqueue {
        return Err(AppError::Other(
            "no queue-operation enqueue lines in transcript — format may have changed".into(),
        ));
    }
    // Every prompt happened at or before the cutoff: the rewound-to turn is
    // the last one, so the fork keeps every fully-written line.
    Ok(offset)
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

    /// Minimal synthetic transcript with three turns. This test doubles as the
    /// format smoke test: it encodes the exact line shapes the fork relies on,
    /// and it must be updated deliberately if a claude release changes them.
    fn synthetic() -> String {
        [
            enqueue("2026-09-02T10:00:00.000Z", "turn one"),
            dequeue("2026-09-02T10:00:00.100Z"),
            r#"{"type":"user","message":{"role":"user","content":"turn one"}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[]}}"#.into(),
            enqueue("2026-09-02T11:00:00.000Z", "turn two"),
            dequeue("2026-09-02T11:00:00.100Z"),
            r#"{"type":"user","message":{"role":"user","content":"turn two"}}"#.into(),
            enqueue("2026-09-02T12:00:00.000Z", "turn three"),
            dequeue("2026-09-02T12:00:00.100Z"),
            r#"{"type":"user","message":{"role":"user","content":"turn three"}}"#.into(),
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
