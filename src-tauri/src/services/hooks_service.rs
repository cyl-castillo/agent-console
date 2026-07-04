use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use parking_lot::Mutex;
use std::thread;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};
use crate::services::activity_service::ActivityEvent;
use crate::services::snapshot_service;
use crate::state::AppState;

/// Bundled hook scripts written to disk at runtime.
const USERPROMPT_HOOK: &str = include_str!("../../resources/userprompt-hook.cjs");
const PRETOOLUSE_HOOK: &str = include_str!("../../resources/pretooluse-hook.cjs");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksStatus {
    pub session_dir: PathBuf,
    pub script_path: PathBuf,
    pub pretooluse_script_path: PathBuf,
    pub installed: bool,
    pub pretooluse_installed: bool,
    pub settings_path: PathBuf,
}

pub struct HooksRuntime {
    session_dir: PathBuf,
    script_path: PathBuf,
    pretooluse_script_path: PathBuf,
    watcher_started: Mutex<bool>,
    approvals_watcher_started: Mutex<bool>,
}

impl HooksRuntime {
    pub fn new() -> AppResult<Self> {
        let cache = dirs::cache_dir()
            .ok_or_else(|| AppError::Other("no cache dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&cache)?;
        let session_dir = cache
            .join("sessions")
            .join(format!("{}", std::process::id()));
        fs::create_dir_all(&session_dir)?;
        fs::create_dir_all(session_dir.join("approvals"))?;
        let script_path = ensure_hook_script(&cache, "userprompt-hook.cjs", USERPROMPT_HOOK)?;
        let pretooluse_script_path =
            ensure_hook_script(&cache, "pretooluse-hook.cjs", PRETOOLUSE_HOOK)?;
        Ok(Self {
            session_dir,
            script_path,
            pretooluse_script_path,
            watcher_started: Mutex::new(false),
            approvals_watcher_started: Mutex::new(false),
        })
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }
    #[allow(dead_code)]
    pub fn script_path(&self) -> &Path {
        &self.script_path
    }

    /// Start the events.jsonl watcher (idempotent).
    pub fn start_watcher(&self, app: AppHandle) {
        let mut started = self.watcher_started.lock();
        if !*started {
            *started = true;
            let dir = self.session_dir.clone();
            let app2 = app.clone();
            thread::spawn(move || watcher_loop(app2, dir));
        }
        drop(started);

        let mut started2 = self.approvals_watcher_started.lock();
        if !*started2 {
            *started2 = true;
            let dir = self.session_dir.join("approvals");
            thread::spawn(move || approvals_watcher_loop(app, dir));
        }
    }

    pub fn status(&self) -> HooksStatus {
        let settings_path = settings_path();
        let installed = is_hook_installed(&settings_path, "UserPromptSubmit", &self.script_path)
            .unwrap_or(false);
        let pretooluse_installed =
            is_hook_installed(&settings_path, "PreToolUse", &self.pretooluse_script_path)
                .unwrap_or(false);
        HooksStatus {
            session_dir: self.session_dir.clone(),
            script_path: self.script_path.clone(),
            pretooluse_script_path: self.pretooluse_script_path.clone(),
            installed,
            pretooluse_installed,
            settings_path,
        }
    }

    /// Merge UserPromptSubmit + PreToolUse hooks into ~/.claude/settings.json.
    pub fn install(&self) -> AppResult<HooksStatus> {
        let settings_path = settings_path();
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut settings: Value = if settings_path.exists() {
            let txt = fs::read_to_string(&settings_path)?;
            serde_json::from_str(&txt).unwrap_or(json!({}))
        } else {
            json!({})
        };
        if !settings.is_object() {
            settings = json!({});
        }

        upsert_hook(&mut settings, "UserPromptSubmit", &self.script_path);
        upsert_hook(&mut settings, "PreToolUse", &self.pretooluse_script_path);

        write_settings_atomic(&settings_path, &settings)?;
        Ok(self.status())
    }

    /// Auto-install the lightweight UserPromptSubmit observer on first run, so
    /// the features that depend on it (session-name suggestions, resume binding,
    /// activity stream, auto-snapshots) work out of the box — not only after the
    /// user finds and flips the integration toggle. This was the gap behind
    /// "the name recommender only works on my machine": on a clean install the
    /// hook was never registered.
    ///
    /// Guarded by a one-time marker so a deliberate `uninstall()` sticks across
    /// restarts (we never re-add it). The PreToolUse permission bridge is NOT
    /// auto-installed — it changes how Claude runs and stays opt-in. Snapshots
    /// are safe to enable by default: they're written to a private ref via
    /// write-tree/commit-tree and never touch HEAD, the index, or the worktree.
    pub fn ensure_autoinstalled(&self) -> AppResult<()> {
        let marker = self.script_path.with_file_name(".userprompt-autoinstalled");
        if marker.exists() {
            return Ok(());
        }
        let settings_path = settings_path();
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut settings: Value = if settings_path.exists() {
            serde_json::from_str(&fs::read_to_string(&settings_path)?).unwrap_or(json!({}))
        } else {
            json!({})
        };
        if !settings.is_object() {
            settings = json!({});
        }
        upsert_hook(&mut settings, "UserPromptSubmit", &self.script_path);
        write_settings_atomic(&settings_path, &settings)?;
        // Best-effort marker: if it fails we'd re-run the idempotent upsert next
        // launch, which is harmless.
        let _ = fs::write(&marker, b"1");
        Ok(())
    }

    /// Remove our hook entries from settings.json. Other hooks/settings untouched.
    pub fn uninstall(&self) -> AppResult<HooksStatus> {
        let settings_path = settings_path();
        if !settings_path.exists() {
            return Ok(self.status());
        }
        let txt = fs::read_to_string(&settings_path)?;
        let mut settings: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
        if let Some(hooks) = settings.get_mut("hooks").and_then(|v| v.as_object_mut()) {
            for key in ["UserPromptSubmit", "PreToolUse"] {
                let target = if key == "UserPromptSubmit" {
                    &self.script_path
                } else {
                    &self.pretooluse_script_path
                };
                if let Some(arr) = hooks.get_mut(key).and_then(|v| v.as_array_mut()) {
                    arr.retain(|e| !has_command_path(e, target));
                }
            }
        }
        write_settings_atomic(&settings_path, &settings)?;
        Ok(self.status())
    }

    /// Write a response for an in-flight approval. The hook script is polling
    /// for <session_dir>/approvals/<id>.res.json and will pick this up.
    pub fn respond(&self, id: &str, decision: &str, reason: Option<&str>) -> AppResult<()> {
        if !["allow", "deny", "ask"].contains(&decision) {
            return Err(AppError::InvalidArgument(format!(
                "bad decision: {decision}"
            )));
        }
        let dir = self.session_dir.join("approvals");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{id}.res.json"));
        let body = json!({ "id": id, "decision": decision, "reason": reason });
        // Write to a temp file then rename, so the polling hook never reads a partial JSON.
        let tmp = dir.join(format!("{id}.res.tmp"));
        fs::write(&tmp, body.to_string())?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Write settings.json via temp file + rename so a crash mid-write can never
/// leave ~/.claude/settings.json truncated or half-serialized.
fn write_settings_atomic(settings_path: &Path, settings: &Value) -> AppResult<()> {
    let tmp = settings_path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(settings).unwrap())?;
    fs::rename(&tmp, settings_path)?;
    Ok(())
}

fn upsert_hook(settings: &mut Value, event: &str, script_path: &Path) {
    let hooks = settings.get("hooks").cloned().unwrap_or(json!({}));
    let mut hooks = if hooks.is_object() { hooks } else { json!({}) };
    let entry = json!({
        "matcher": "*",
        "hooks": [{ "type": "command", "command": script_path.to_string_lossy() }]
    });
    let arr = hooks
        .get_mut(event)
        .and_then(|v| v.as_array_mut())
        .cloned()
        .unwrap_or_default();
    let already = arr.iter().any(|e| has_command_path(e, script_path));
    let mut new_arr = arr;
    if !already {
        new_arr.push(entry);
    }
    hooks
        .as_object_mut()
        .unwrap()
        .insert(event.to_string(), Value::Array(new_arr));
    settings
        .as_object_mut()
        .unwrap()
        .insert("hooks".to_string(), hooks);
}

fn settings_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".claude/settings.json"))
        .unwrap_or_else(|| PathBuf::from(".claude/settings.json"))
}

fn ensure_hook_script(runtime_dir: &Path, filename: &str, body: &str) -> AppResult<PathBuf> {
    let path = runtime_dir.join(filename);
    fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&path)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&path, perm)?;
    }
    Ok(path)
}

fn is_hook_installed(settings_path: &Path, event: &str, script_path: &Path) -> AppResult<bool> {
    if !settings_path.exists() {
        return Ok(false);
    }
    let txt = fs::read_to_string(settings_path)?;
    let v: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
    let Some(arr) = v
        .pointer(&format!("/hooks/{event}"))
        .and_then(|v| v.as_array())
    else {
        return Ok(false);
    };
    Ok(arr.iter().any(|e| has_command_path(e, script_path)))
}

fn has_command_path(entry: &Value, target: &Path) -> bool {
    let target_str = target.to_string_lossy().to_string();
    let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) else {
        return false;
    };
    hooks.iter().any(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .map(|c| c == target_str)
            .unwrap_or(false)
    })
}

/// Watch a directory and return (watcher, wake-receiver, wait-interval).
///
/// Preferred mode: a platform filesystem watcher wakes the loop the moment
/// something changes, with a slow 2s heartbeat as a safety net for missed
/// events. If the watcher can't start (inotify exhaustion, exotic FS), fall
/// back to pure polling at `poll_fallback` — the pre-notify behavior.
fn dir_wake_source(
    dir: &Path,
    poll_fallback: Duration,
) -> (
    Option<notify::RecommendedWatcher>,
    mpsc::Receiver<()>,
    Duration,
) {
    let (tx, rx) = mpsc::channel::<()>();
    let watcher = notify::recommended_watcher(move |_res| {
        let _ = tx.send(());
    })
    .ok()
    .and_then(|mut w| match w.watch(dir, RecursiveMode::NonRecursive) {
        Ok(()) => Some(w),
        Err(e) => {
            eprintln!("hooks: fs watch on {} failed, polling: {e}", dir.display());
            None
        }
    });
    let interval = if watcher.is_some() {
        Duration::from_secs(2)
    } else {
        poll_fallback
    };
    (watcher, rx, interval)
}

/// Block until the next filesystem wake-up (or the heartbeat), then coalesce
/// any burst of queued wake-ups into this one pass.
fn await_wake(rx: &mpsc::Receiver<()>, interval: Duration, watching: bool) {
    if watching {
        let _ = rx.recv_timeout(interval);
        while rx.try_recv().is_ok() {}
    } else {
        thread::sleep(interval);
    }
}

/// Parse JSONL appended to `path` past `last_size`. Returns the events and the
/// new read offset. Only complete lines (through the last `\n`) are consumed —
/// a torn tail mid-append stays buffered for the next pass instead of being
/// half-parsed and dropped. A shrunken file (truncation) resets to 0 so the
/// next pass reprocesses from the start.
fn drain_new_lines(path: &Path, last_size: u64) -> (Vec<Value>, u64) {
    let Ok(meta) = fs::metadata(path) else {
        return (Vec::new(), last_size);
    };
    let size = meta.len();
    if size == last_size {
        return (Vec::new(), last_size);
    }
    if size < last_size {
        return (Vec::new(), 0);
    }
    let Ok(mut f) = fs::File::open(path) else {
        return (Vec::new(), last_size);
    };
    if f.seek(SeekFrom::Start(last_size)).is_err() {
        return (Vec::new(), last_size);
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return (Vec::new(), last_size);
    }
    let Some(consumed) = buf.rfind('\n').map(|i| i + 1) else {
        return (Vec::new(), last_size);
    };
    let events = buf[..consumed]
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    (events, last_size + consumed as u64)
}

fn watcher_loop(app: AppHandle, dir: PathBuf) {
    let events_file = dir.join("events.jsonl");
    let (watcher, rx, interval) = dir_wake_source(&dir, Duration::from_millis(120));
    let mut last_size: u64 = 0;
    loop {
        await_wake(&rx, interval, watcher.is_some());
        let (events, next) = drain_new_lines(&events_file, last_size);
        last_size = next;
        for v in &events {
            handle_event(v, &app);
        }
    }
}

fn handle_event(v: &Value, app: &AppHandle) {
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let _ = app.emit(&format!("hook://{kind}"), v);

    if kind == "user_prompt" {
        let state = app.state::<AppState>();
        let project = state.inner.lock().project.clone();
        if let Some(p) = project {
            // Persist the prompt to the durable per-project ledger that
            // "learning mode" reflects over. Best-effort: a failed append must
            // never block the snapshot or the UI event.
            let root = p.root.to_string_lossy();
            let ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
            let _ = state.activity.record(
                root.as_ref(),
                &ActivityEvent {
                    ts,
                    kind: "user_prompt".into(),
                    prompt: str_field(v, "prompt"),
                    skill: str_field(v, "skill"),
                    term_id: str_field(v, "termId"),
                    session_id: str_field(v, "sessionId"),
                    snapshot_sha: None,
                },
            );

            // Auto-snapshot the working tree the prompt ran in. Worktree
            // sessions report their own cwd — snapshotting the project root
            // there would checkpoint the wrong checkout. Fall back to the
            // project root for events without a cwd (older hook script).
            let snap_repo = str_field(v, "cwd")
                .map(PathBuf::from)
                .filter(|c| c.is_dir())
                .unwrap_or_else(|| p.root.clone());
            let id = uuid::Uuid::new_v4().to_string();
            if let Ok(Some(snap)) = snapshot_service::create(&snap_repo, &id) {
                // Record the snapshot too, so reflection can correlate a prompt
                // with the working-tree checkpoint it produced.
                let _ = state.activity.record(
                    root.as_ref(),
                    &ActivityEvent {
                        ts,
                        kind: "snapshot".into(),
                        prompt: None,
                        skill: None,
                        term_id: str_field(v, "termId"),
                        session_id: str_field(v, "sessionId"),
                        snapshot_sha: Some(snap.commit_sha.clone()),
                    },
                );
                let _ = app.emit("snapshot://created", &snap);
            }
        }
    }
}

/// Read an optional string field from a hook payload, treating empty strings as
/// absent so the ledger doesn't store noise like `"skill": ""`.
fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn approvals_watcher_loop(app: AppHandle, dir: PathBuf) {
    let (watcher, rx, interval) = dir_wake_source(&dir, Duration::from_millis(100));
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        await_wake(&rx, interval, watcher.is_some());
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut current: HashSet<String> = HashSet::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".req.json") {
                continue;
            }
            current.insert(name.clone());
            if seen.contains(&name) {
                continue;
            }
            let Ok(txt) = fs::read_to_string(e.path()) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&txt) else {
                continue;
            };
            let _ = app.emit("approval://request", &v);
            seen.insert(name);
        }
        // Drop ids that disappeared (hook cleaned them up).
        seen.retain(|n| current.contains(n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn drain_consumes_complete_lines_incrementally() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ac-hooks-drain-{nanos}.jsonl"));

        // Missing file: nothing to do, offset unchanged.
        let (evs, off) = drain_new_lines(&path, 0);
        assert!(evs.is_empty());
        assert_eq!(off, 0);

        // Two complete events arrive.
        fs::write(&path, "{\"type\":\"a\"}\n{\"type\":\"b\"}\n").unwrap();
        let (evs, off) = drain_new_lines(&path, 0);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[1]["type"], "b");

        // No growth: nothing new.
        let (evs, off2) = drain_new_lines(&path, off);
        assert!(evs.is_empty());
        assert_eq!(off2, off);

        // A torn append (no trailing newline yet) must NOT be consumed…
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"type\":\"c\"").unwrap();
        f.flush().unwrap();
        let (evs, off3) = drain_new_lines(&path, off);
        assert!(evs.is_empty(), "half-written line stays buffered");
        assert_eq!(off3, off, "offset must not advance past torn tail");

        // …and is delivered intact once the writer finishes the line.
        f.write_all(b"}\n").unwrap();
        f.flush().unwrap();
        let (evs, off4) = drain_new_lines(&path, off3);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["type"], "c");
        assert!(off4 > off3);

        // Blank and malformed lines are skipped, valid neighbors still parse.
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"\nnot json\n{\"type\":\"d\"}\n").unwrap();
        drop(f);
        let (evs, off5) = drain_new_lines(&path, off4);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["type"], "d");

        // Truncation resets the offset so the next pass starts over.
        fs::write(&path, "{\"type\":\"fresh\"}\n").unwrap();
        let (evs, off6) = drain_new_lines(&path, off5);
        assert!(evs.is_empty(), "shrink pass only resets");
        assert_eq!(off6, 0);
        let (evs, _) = drain_new_lines(&path, off6);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["type"], "fresh");

        let _ = fs::remove_file(&path);
    }
}
