use std::fs::{self, OpenOptions};

use parking_lot::Mutex;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// One durable record of something the user did inside a project, captured from
/// the UserPromptSubmit hook stream (and the auto-snapshots it triggers).
///
/// This is the substrate "learning mode" reflects over. The in-memory prompt
/// buffer (skillsStore) and the per-PID `events.jsonl` are both lost on restart,
/// so to learn from *daily* work we persist the signal here: append-only and
/// keyed by project, so a reflection pass can read back a real window of history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    /// Epoch milliseconds (taken from the hook payload when present, else 0).
    pub ts: i64,
    /// Event kind, mirroring the hook `type`: "user_prompt", "snapshot", ...
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Slash-command detected at the start of a prompt, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Snapshot commit sha when kind == "snapshot".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_sha: Option<String>,
}

/// Grow past this size and the ledger is trimmed to the most recent events. A
/// user prompt is a few hundred bytes, so ~8 MB is many thousands of events —
/// generous for "learn from recent work" without unbounded growth.
const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Events retained after a trim.
const KEEP_EVENTS: usize = 4000;

pub struct ActivityService {
    lock: Mutex<()>,
}

impl ActivityService {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console")
            .join("activity");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn path(project_root: &str) -> AppResult<PathBuf> {
        Ok(Self::dir()?.join(crate::services::persistence::project_file_key(project_root)))
    }

    /// Append one event. Best-effort durability: a torn final line from a crash
    /// mid-append is tolerated by `list` (it skips unparseable lines), so we
    /// favor a cheap append over a full atomic rewrite on every event.
    pub fn record(&self, project_root: &str, event: &ActivityEvent) -> AppResult<()> {
        let _g = self.lock.lock();
        let path = Self::path(project_root)?;
        let mut line = serde_json::to_string(event)
            .map_err(|e| AppError::Other(format!("serialize activity: {e}")))?;
        line.push('\n');
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        f.write_all(line.as_bytes())?;
        drop(f);
        // Trim only once the file crosses the cap, so the common path stays a
        // single append with no read-back.
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() > MAX_BYTES {
                let _ = Self::trim(&path);
            }
        }
        Ok(())
    }

    /// Rewrite the ledger keeping only the most recent KEEP_EVENTS (shared
    /// atomic trim from the persistence module).
    fn trim(path: &Path) -> AppResult<()> {
        crate::services::persistence::trim_jsonl(path, KEEP_EVENTS)
    }

    /// Most recent events in chronological order (oldest first), capped at
    /// `limit` when given. Unparseable lines (e.g. a crash-torn tail) are
    /// skipped rather than failing the whole read.
    pub fn list(&self, project_root: &str, limit: Option<usize>) -> AppResult<Vec<ActivityEvent>> {
        let _g = self.lock.lock();
        let path = Self::path(project_root)?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path)?;
        let mut events: Vec<ActivityEvent> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<ActivityEvent>(l).ok())
            .collect();
        if let Some(n) = limit {
            if events.len() > n {
                events = events.split_off(events.len() - n);
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn ev(ts: i64, prompt: &str) -> ActivityEvent {
        ActivityEvent {
            ts,
            kind: "user_prompt".into(),
            prompt: Some(prompt.into()),
            skill: None,
            term_id: Some("t1".into()),
            session_id: None,
            snapshot_sha: None,
        }
    }

    /// Exercises the real record/list code in an isolated data dir (via
    /// XDG_DATA_HOME, which dirs::data_local_dir respects) so the user's real
    /// ledger is never touched. One test fn on purpose: it mutates a
    /// process-global env var, so it must not race a sibling test.
    #[test]
    fn ledger_persists_and_isolates_by_project() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-activity-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = ActivityService::new();
        let a = "/proj/a";
        let b = "/proj/b";

        // 1. Fresh project: empty, not an error.
        assert!(svc.list(a, None).unwrap().is_empty());

        // 2. Append preserves order; a second project is fully isolated.
        svc.record(a, &ev(1, "first")).unwrap();
        svc.record(a, &ev(2, "second")).unwrap();
        svc.record(b, &ev(9, "other-project")).unwrap();
        let got = svc.list(a, None).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].prompt.as_deref(), Some("first"));
        assert_eq!(got[1].prompt.as_deref(), Some("second"));
        assert_eq!(
            svc.list(b, None).unwrap().len(),
            1,
            "projects must not bleed"
        );

        // 3. limit returns the most recent N, still chronological.
        svc.record(a, &ev(3, "third")).unwrap();
        let last2 = svc.list(a, Some(2)).unwrap();
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[0].prompt.as_deref(), Some("second"));
        assert_eq!(last2[1].prompt.as_deref(), Some("third"));

        // 4. A crash-torn final line is skipped, not fatal.
        let path = ActivityService::path(a).unwrap();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{ this is half a line").unwrap();
        drop(f);
        assert_eq!(
            svc.list(a, None).unwrap().len(),
            3,
            "torn tail line is ignored"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
