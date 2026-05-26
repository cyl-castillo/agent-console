use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};
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
        let session_dir = cache.join("sessions").join(format!("{}", std::process::id()));
        fs::create_dir_all(&session_dir)?;
        fs::create_dir_all(session_dir.join("approvals"))?;
        let script_path = ensure_hook_script(&cache, "userprompt-hook.cjs", USERPROMPT_HOOK)?;
        let pretooluse_script_path = ensure_hook_script(&cache, "pretooluse-hook.cjs", PRETOOLUSE_HOOK)?;
        Ok(Self {
            session_dir,
            script_path,
            pretooluse_script_path,
            watcher_started: Mutex::new(false),
            approvals_watcher_started: Mutex::new(false),
        })
    }

    pub fn session_dir(&self) -> &Path { &self.session_dir }
    pub fn script_path(&self) -> &Path { &self.script_path }

    /// Start the events.jsonl watcher (idempotent).
    pub fn start_watcher(&self, app: AppHandle) {
        let mut started = self.watcher_started.lock().unwrap();
        if !*started {
            *started = true;
            let dir = self.session_dir.clone();
            let app2 = app.clone();
            thread::spawn(move || watcher_loop(app2, dir));
        }
        drop(started);

        let mut started2 = self.approvals_watcher_started.lock().unwrap();
        if !*started2 {
            *started2 = true;
            let dir = self.session_dir.join("approvals");
            thread::spawn(move || approvals_watcher_loop(app, dir));
        }
    }

    pub fn status(&self) -> HooksStatus {
        let settings_path = settings_path();
        let installed = is_hook_installed(&settings_path, "UserPromptSubmit", &self.script_path).unwrap_or(false);
        let pretooluse_installed = is_hook_installed(&settings_path, "PreToolUse", &self.pretooluse_script_path).unwrap_or(false);
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
        if let Some(parent) = settings_path.parent() { fs::create_dir_all(parent)?; }

        let mut settings: Value = if settings_path.exists() {
            let txt = fs::read_to_string(&settings_path)?;
            serde_json::from_str(&txt).unwrap_or(json!({}))
        } else { json!({}) };
        if !settings.is_object() { settings = json!({}); }

        upsert_hook(&mut settings, "UserPromptSubmit", &self.script_path);
        upsert_hook(&mut settings, "PreToolUse", &self.pretooluse_script_path);

        fs::write(&settings_path, serde_json::to_string_pretty(&settings).unwrap())?;
        Ok(self.status())
    }

    /// Remove our hook entries from settings.json. Other hooks/settings untouched.
    pub fn uninstall(&self) -> AppResult<HooksStatus> {
        let settings_path = settings_path();
        if !settings_path.exists() { return Ok(self.status()); }
        let txt = fs::read_to_string(&settings_path)?;
        let mut settings: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
        if let Some(hooks) = settings.get_mut("hooks").and_then(|v| v.as_object_mut()) {
            for key in ["UserPromptSubmit", "PreToolUse"] {
                let target = if key == "UserPromptSubmit" { &self.script_path } else { &self.pretooluse_script_path };
                if let Some(arr) = hooks.get_mut(key).and_then(|v| v.as_array_mut()) {
                    arr.retain(|e| !has_command_path(e, target));
                }
            }
        }
        fs::write(&settings_path, serde_json::to_string_pretty(&settings).unwrap())?;
        Ok(self.status())
    }

    /// Write a response for an in-flight approval. The hook script is polling
    /// for <session_dir>/approvals/<id>.res.json and will pick this up.
    pub fn respond(&self, id: &str, decision: &str, reason: Option<&str>) -> AppResult<()> {
        if !["allow", "deny", "ask"].contains(&decision) {
            return Err(AppError::InvalidArgument(format!("bad decision: {decision}")));
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

fn upsert_hook(settings: &mut Value, event: &str, script_path: &Path) {
    let hooks = settings.get("hooks").cloned().unwrap_or(json!({}));
    let mut hooks = if hooks.is_object() { hooks } else { json!({}) };
    let entry = json!({
        "matcher": "*",
        "hooks": [{ "type": "command", "command": script_path.to_string_lossy() }]
    });
    let arr = hooks.get_mut(event)
        .and_then(|v| v.as_array_mut())
        .cloned()
        .unwrap_or_default();
    let already = arr.iter().any(|e| has_command_path(e, script_path));
    let mut new_arr = arr;
    if !already { new_arr.push(entry); }
    hooks.as_object_mut().unwrap().insert(event.to_string(), Value::Array(new_arr));
    settings.as_object_mut().unwrap().insert("hooks".to_string(), hooks);
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
    if !settings_path.exists() { return Ok(false); }
    let txt = fs::read_to_string(settings_path)?;
    let v: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
    let Some(arr) = v.pointer(&format!("/hooks/{event}")).and_then(|v| v.as_array()) else {
        return Ok(false);
    };
    Ok(arr.iter().any(|e| has_command_path(e, script_path)))
}

fn has_command_path(entry: &Value, target: &Path) -> bool {
    let target_str = target.to_string_lossy().to_string();
    let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) else { return false };
    hooks.iter().any(|h| {
        h.get("command").and_then(|c| c.as_str()).map(|c| c == target_str).unwrap_or(false)
    })
}

fn watcher_loop(app: AppHandle, dir: PathBuf) {
    let events_file = dir.join("events.jsonl");
    let mut last_size: u64 = 0;
    loop {
        thread::sleep(Duration::from_millis(120));
        let Ok(meta) = fs::metadata(&events_file) else { continue };
        let size = meta.len();
        if size == last_size { continue; }
        if size < last_size { last_size = 0; continue; }

        let Ok(content) = fs::read_to_string(&events_file) else { continue };
        if (content.len() as u64) < last_size { last_size = 0; continue; }
        let tail = &content[(last_size as usize).min(content.len())..];
        for line in tail.lines() {
            if line.trim().is_empty() { continue; }
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            handle_event(&v, &app);
        }
        last_size = size;
    }
}

fn handle_event(v: &Value, app: &AppHandle) {
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let _ = app.emit(&format!("hook://{kind}"), v);

    if kind == "user_prompt" {
        // Auto-snapshot in the active project's working tree.
        let state = app.state::<AppState>();
        let project = state.inner.lock().unwrap().project.clone();
        if let Some(p) = project {
            let id = uuid::Uuid::new_v4().to_string();
            if let Ok(Some(snap)) = snapshot_service::create(&p.root, &id) {
                let _ = app.emit("snapshot://created", &snap);
            }
        }
    }
}

fn approvals_watcher_loop(app: AppHandle, dir: PathBuf) {
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        thread::sleep(Duration::from_millis(100));
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        let mut current: HashSet<String> = HashSet::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".req.json") { continue; }
            current.insert(name.clone());
            if seen.contains(&name) { continue; }
            let Ok(txt) = fs::read_to_string(e.path()) else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&txt) else { continue };
            let _ = app.emit("approval://request", &v);
            seen.insert(name);
        }
        // Drop ids that disappeared (hook cleaned them up).
        seen.retain(|n| current.contains(n));
    }
}
