//! Flywheel metrics (E4 of the knowledge flywheel).
//!
//! Answers "is the corpus actually learning?" with numbers instead of vibes:
//! how big the corpus is, how often injection fires (the 30-day curve), and
//! whether what gets injected is judged useful — with the honesty guard that
//! a usefulness rate over few verdicts is labeled by its coverage.
//!
//! The durable piece is the injection log: a per-project JSONL (same key and
//! trim discipline as the activity ledger) appended on every served
//! injection. Day bucketing happens against LOCAL day boundaries computed by
//! the frontend — the repo's standing anti-day-flip rule; the backend never
//! guesses timezones.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::corpus_feedback;
use crate::services::persistence::{project_file_key, trim_jsonl};
use crate::services::semantic_index;

/// Injection log retention. ~5k injections is months of heavy use; the
/// metrics window is 30 days, so this is comfortable headroom.
const LOG_KEEP: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogEntry {
    ts: u64,
    /// How many docs shipped in this injection (profile included).
    docs: u32,
}

fn log_dir() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console")
        .join("inject-log");
    fs::create_dir_all(&dir)?;
    Ok(dir.join(""))
}

fn log_path(project_root: &str) -> AppResult<PathBuf> {
    Ok(log_dir()?.join(project_file_key(project_root)))
}

/// Append one injection to the durable log. Best-effort by contract: the
/// inject path never fails over bookkeeping.
pub fn record_injection(project_root: &str, ts_ms: u64, docs: u32) {
    let Ok(path) = log_path(project_root) else {
        return;
    };
    let Ok(line) = serde_json::to_string(&LogEntry { ts: ts_ms, docs }) else {
        return;
    };
    let appended = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{line}"));
    if appended.is_ok() {
        let _ = trim_jsonl(&path, LOG_KEEP);
    }
}

/// All logged injection timestamps, oldest first. Tolerant: unparsable lines
/// are skipped, a missing file is an empty history.
fn read_timestamps(project_root: &str) -> Vec<u64> {
    let Ok(path) = log_path(project_root) else {
        return Vec::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<LogEntry>(l).ok())
        .map(|e| e.ts)
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DayBucket {
    pub start_ms: i64,
    pub count: u32,
}

/// Bucket timestamps into consecutive [starts[i], starts[i+1]) windows.
/// `day_starts` are ascending local-midnight boundaries from the frontend —
/// N+1 boundaries produce N buckets; anything outside the range is ignored.
fn bucket_by_day(timestamps: &[u64], day_starts: &[i64]) -> Vec<DayBucket> {
    if day_starts.len() < 2 {
        return Vec::new();
    }
    let mut buckets: Vec<DayBucket> = day_starts[..day_starts.len() - 1]
        .iter()
        .map(|s| DayBucket {
            start_ms: *s,
            count: 0,
        })
        .collect();
    for &ts in timestamps {
        let ts = ts as i64;
        if ts < day_starts[0] || ts >= day_starts[day_starts.len() - 1] {
            continue;
        }
        // partition_point: first boundary strictly greater than ts → its
        // predecessor's window owns the timestamp.
        let idx = day_starts.partition_point(|s| *s <= ts) - 1;
        buckets[idx].count += 1;
    }
    buckets
}

/// helpful/(helpful+unhelpful), or None when nobody has verdicted anything —
/// a rate over zero votes is a lie, not a number.
fn usefulness_pct(helpful: u32, unhelpful: u32) -> Option<f32> {
    let total = helpful + unhelpful;
    if total == 0 {
        return None;
    }
    Some(helpful as f32 * 100.0 / total as f32)
}

/// Share of injected docs that received at least one verdict. Low coverage
/// means the usefulness rate rests on thin evidence — the GUI says so.
fn coverage_pct(docs_injected: u32, docs_verdicted: u32) -> Option<f32> {
    if docs_injected == 0 {
        return None;
    }
    Some(docs_verdicted as f32 * 100.0 / docs_injected as f32)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlywheelMetrics {
    pub corpus_memories: usize,
    pub corpus_skills: usize,
    pub excluded_docs: u32,
    pub injections_total: usize,
    pub days: Vec<DayBucket>,
    pub helpful_total: u32,
    pub unhelpful_total: u32,
    pub usefulness_pct: Option<f32>,
    pub verdict_coverage_pct: Option<f32>,
}

/// The full metrics picture for one project. `day_starts` are the local-day
/// boundaries for the curve (ascending, N+1 for N days).
pub fn metrics(project_root: &str, day_starts: &[i64]) -> FlywheelMetrics {
    // Corpus truth = what injection can draw from (sources, not the possibly
    // stale index). Best-effort: an unreadable corpus reads as empty.
    let (corpus_memories, corpus_skills) = semantic_index::project_sources(project_root)
        .map(|docs| {
            let mem = docs.iter().filter(|d| d.kind == "memory").count();
            (mem, docs.len() - mem)
        })
        .unwrap_or((0, 0));

    let stats = corpus_feedback::stats(project_root);
    let excluded_docs = stats.values().filter(|s| s.excluded()).count() as u32;
    let helpful_total: u32 = stats.values().map(|s| s.helpful).sum();
    let unhelpful_total: u32 = stats.values().map(|s| s.unhelpful).sum();
    let docs_injected = stats.values().filter(|s| s.injected_count > 0).count() as u32;
    let docs_verdicted = stats
        .values()
        .filter(|s| s.injected_count > 0 && (s.helpful > 0 || s.unhelpful > 0))
        .count() as u32;

    let timestamps = read_timestamps(project_root);
    FlywheelMetrics {
        corpus_memories,
        corpus_skills,
        excluded_docs,
        injections_total: timestamps.len(),
        days: bucket_by_day(&timestamps, day_starts),
        helpful_total,
        unhelpful_total,
        usefulness_pct: usefulness_pct(helpful_total, unhelpful_total),
        verdict_coverage_pct: coverage_pct(docs_injected, docs_verdicted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_respect_boundaries_and_ignore_out_of_range() {
        // Three days: [0,100), [100,200), [200,300).
        let starts = vec![0i64, 100, 200, 300];
        let ts = vec![5u64, 99, 100, 250, 299, 300, 1000];
        let got = bucket_by_day(&ts, &starts);
        assert_eq!(got.len(), 3);
        assert_eq!((got[0].start_ms, got[0].count), (0, 2), "5 and 99");
        assert_eq!(
            (got[1].start_ms, got[1].count),
            (100, 1),
            "boundary ts=100 goes RIGHT"
        );
        assert_eq!(
            (got[2].start_ms, got[2].count),
            (200, 2),
            "250, 299; 300+ ignored"
        );
        assert!(
            bucket_by_day(&ts, &[42]).is_empty(),
            "one boundary = no window"
        );
    }

    #[test]
    fn rates_refuse_to_divide_by_zero() {
        assert_eq!(usefulness_pct(0, 0), None);
        assert_eq!(usefulness_pct(3, 1), Some(75.0));
        assert_eq!(coverage_pct(0, 0), None);
        assert_eq!(coverage_pct(4, 1), Some(25.0));
    }

    #[test]
    fn log_roundtrip_trims_and_tolerates_corruption() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("ac-flywheel-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let root = "/proj/fw";
        assert!(read_timestamps(root).is_empty(), "no log yet, no error");
        record_injection(root, 111, 2);
        record_injection(root, 222, 1);
        assert_eq!(read_timestamps(root), vec![111, 222]);

        // A corrupt line in the middle is skipped, neighbors survive.
        let path = log_path(root).unwrap();
        let mut raw = fs::read_to_string(&path).unwrap();
        raw.insert_str(raw.find('\n').unwrap() + 1, "{ garbage\n");
        fs::write(&path, raw).unwrap();
        assert_eq!(read_timestamps(root), vec![111, 222]);

        // Metrics over the log + empty corpus behave.
        let m = metrics(root, &[0, 500]);
        assert_eq!(m.injections_total, 2);
        assert_eq!(m.days.len(), 1);
        assert_eq!(m.days[0].count, 2);
        assert_eq!(m.usefulness_pct, None, "no verdicts yet");
    }
}
