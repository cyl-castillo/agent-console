use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};

/// Coordinates per-tool approval between the bundled PreToolUse hook script
/// and Agent Console's UI via a shared directory of req-*.json / res-*.json files.
pub struct PermissionBridge {
    dir: Mutex<Option<PathBuf>>,
}

impl Default for PermissionBridge {
    fn default() -> Self { Self { dir: Mutex::new(None) } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub id: String,
    pub tool_name: String,
    pub tool_input: Value,
}

impl PermissionBridge {
    pub fn new() -> Self { Self::default() }

    /// Ensure a per-session directory exists and a poller thread is watching it.
    /// Returns the path to pass as AGENT_CONSOLE_HOOK_DIR to the agent process.
    pub fn ensure(&self, app: &AppHandle, session_id: &str) -> AppResult<PathBuf> {
        let mut slot = self.dir.lock().unwrap();
        if let Some(p) = slot.as_ref() {
            return Ok(p.clone());
        }
        let base = dirs::cache_dir()
            .ok_or_else(|| AppError::Other("no cache dir".into()))?
            .join("agent-console")
            .join("hooks")
            .join(session_id);
        fs::create_dir_all(&base)?;
        spawn_poller(app.clone(), base.clone());
        *slot = Some(base.clone());
        Ok(base)
    }

    pub fn respond(&self, id: &str, allow: bool, reason: Option<String>) -> AppResult<()> {
        let dir = self.dir.lock().unwrap()
            .clone()
            .ok_or_else(|| AppError::InvalidArgument("permission bridge not initialized".into()))?;
        let body = json!({ "allow": allow, "reason": reason });
        let path = dir.join(format!("res-{id}.json"));
        fs::write(&path, body.to_string())?;
        Ok(())
    }

    /// Flip on the "approve everything for this session" sentinel.
    pub fn set_approve_all(&self, enabled: bool) -> AppResult<()> {
        let dir = self.dir.lock().unwrap()
            .clone()
            .ok_or_else(|| AppError::InvalidArgument("permission bridge not initialized".into()))?;
        let sentinel = dir.join("approve-all");
        if enabled { fs::write(&sentinel, "1")?; }
        else { let _ = fs::remove_file(&sentinel); }
        Ok(())
    }

    /// Wipe the directory (used on session reset).
    pub fn clear(&self) {
        let mut slot = self.dir.lock().unwrap();
        if let Some(dir) = slot.take() {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

/// Filesystem poller — emits a `perm://request` event whenever a fresh req-*.json appears.
/// Poll rate matches the hook's poll rate (~80ms) so latency stays low.
fn spawn_poller(app: AppHandle, dir: PathBuf) {
    thread::spawn(move || {
        let mut seen: std::collections::HashSet<String> = Default::default();
        loop {
            thread::sleep(Duration::from_millis(80));
            if !dir.exists() { break; }
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("req-") || !name.ends_with(".json") { continue; }
                if seen.contains(&name) { continue; }
                seen.insert(name.clone());
                let Ok(txt) = fs::read_to_string(entry.path()) else { continue };
                let Ok(req) = serde_json::from_str::<PermissionRequest>(&txt) else { continue };
                let _ = app.emit("perm://request", &req);
            }
            // Drop "seen" entries whose req file has been deleted (after decision).
            seen.retain(|n| dir.join(n).exists());
        }
    });
}
