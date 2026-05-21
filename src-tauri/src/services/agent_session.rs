use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::services::permission_bridge::PermissionBridge;
use crate::services::snapshot_service::{self, Snapshot};
use crate::services::task_service::AgentMode;

/// Base prompt applied in every mode.
const BASE_PROMPT: &str = "You are embedded inside Agent Console, a minimalist console IDE. \
The user directs you from a chat panel; they also have an integrated terminal and a Changes view \
showing git diff of your edits. Be concise. When you edit files the user will see the diffs.";

/// Mode-specific addendum.
fn mode_prompt(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Plan =>
            "PLAN mode. Analyze the codebase to understand the task. Do not edit files or run commands. \
             Produce a concrete, ordered plan listing the steps you would take, the files you would touch, \
             and the tests you would run. End with a short risk assessment.",
        AgentMode::Build =>
            "BUILD mode. Implement the requested change carefully. Make minimal, targeted edits. \
             Prefer reading first, then editing the smallest possible scope. Run relevant tests when applicable.",
        AgentMode::Debug =>
            "DEBUG mode. Diagnose the issue precisely. Read logs, errors, tests, and recent diffs. \
             Identify the root cause before proposing a fix. Be explicit about what you do not yet know.",
        AgentMode::Review =>
            "REVIEW mode. Examine the current changes (git diff). Identify bugs, regressions, security or \
             clarity issues, and concrete improvements. Do not modify files yourself — produce a written review.",
    }
}

/// PreToolUse hook script bundled with the binary and dropped to disk on first use.
const HOOK_SCRIPT: &str = include_str!("../../resources/pretool-hook.cjs");

struct AgentSession {
    child: Child,
    stdin: ChildStdin,
    mode: AgentMode,
    #[allow(dead_code)]
    cwd: PathBuf,
}

impl AgentSession {
    fn spawn(app: AppHandle, cwd: &Path, hook_dir: &Path, mode: AgentMode) -> AppResult<Self> {
        let runtime_dir = ensure_runtime_dir()?;
        let hook_script = ensure_hook_script(&runtime_dir)?;
        let settings_path = write_session_settings(&runtime_dir, &hook_script)?;

        let permission_flag = match mode {
            AgentMode::Plan | AgentMode::Review => "plan",
            AgentMode::Build | AgentMode::Debug => "default",
        };
        let system_prompt = format!("{BASE_PROMPT}\n\n{}", mode_prompt(mode));

        let mut child = Command::new("claude")
            .arg("-p")
            .args(["--input-format", "stream-json"])
            .args(["--output-format", "stream-json"])
            .arg("--verbose")
            .args(["--permission-mode", permission_flag])
            .args(["--settings", &settings_path.to_string_lossy()])
            .args(["--append-system-prompt", &system_prompt])
            .env("AGENT_CONSOLE_HOOK_DIR", hook_dir)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AppError::Other(format!("claude spawn: {e}")))?;

        let stdin = child.stdin.take()
            .ok_or_else(|| AppError::Other("no stdin".into()))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| AppError::Other("no stdout".into()))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| AppError::Other("no stderr".into()))?;

        {
            let app = app.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() { continue; }
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        let _ = app.emit("chat://debug", line);
                        continue;
                    };
                    handle_event(&v, &app);
                }
                let _ = app.emit("chat://session-ended", ());
            });
        }

        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                eprintln!("[claude stderr] {line}");
            }
        });

        Ok(Self { child, stdin, mode, cwd: cwd.to_path_buf() })
    }

    fn send_user(&mut self, text: &str, constraints: &[String]) -> AppResult<()> {
        let mut full_text = text.to_string();
        if !constraints.is_empty() {
            full_text.push_str("\n\n---\nConstraints for this turn:\n");
            for c in constraints {
                full_text.push_str("- ");
                full_text.push_str(c);
                full_text.push('\n');
            }
        }
        let payload = json!({
            "type": "user",
            "message": { "role": "user", "content": full_text },
        });
        writeln!(self.stdin, "{payload}")
            .map_err(|e| AppError::Other(format!("write stdin: {e}")))?;
        self.stdin.flush()
            .map_err(|e| AppError::Other(format!("flush stdin: {e}")))?;
        Ok(())
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct AgentRegistry {
    session: Mutex<Option<AgentSession>>,
    pub permissions: Arc<PermissionBridge>,
}

impl AgentRegistry {
    pub fn new() -> Self { Self::default() }

    /// Send a user turn. Creates a snapshot first; respawns if mode changed.
    pub fn send(
        &self,
        app: AppHandle,
        cwd: &Path,
        text: String,
        mode: AgentMode,
        constraints: Vec<String>,
    ) -> AppResult<Option<Snapshot>> {
        let turn_id = Uuid::new_v4().to_string();
        let snapshot = snapshot_service::create(cwd, &turn_id)?;

        let mut slot = self.session.lock().unwrap();
        // If mode changed (or no session yet), respawn.
        let needs_respawn = match slot.as_ref() {
            None => true,
            Some(s) => s.mode != mode,
        };
        if needs_respawn {
            if let Some(mut prev) = slot.take() { prev.kill(); }
            let hook_dir = self.permissions.ensure(&app, &turn_id)?;
            *slot = Some(AgentSession::spawn(app.clone(), cwd, &hook_dir, mode)?);
            let _ = app.emit("chat://session-started", json!({ "mode": mode.as_str() }));
        }
        slot.as_mut().unwrap().send_user(&text, &constraints)?;
        Ok(snapshot)
    }

    pub fn reset(&self) {
        if let Some(mut s) = self.session.lock().unwrap().take() { s.kill(); }
        self.permissions.clear();
    }
}

fn handle_event(v: &Value, app: &AppHandle) {
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        "system" => {
            if v.get("subtype").and_then(|s| s.as_str()) == Some("init") {
                let session_id = v.get("session_id").and_then(|s| s.as_str()).unwrap_or("");
                let _ = app.emit("chat://session-init", json!({ "sessionId": session_id }));
            }
        }
        "assistant" => {
            if let Some(blocks) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                for block in blocks { emit_block(block, app); }
            }
        }
        "user" => {
            if let Some(blocks) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        emit_tool_result(block, app);
                    }
                }
            }
        }
        "result" => {
            let cost = v.get("total_cost_usd").and_then(|c| c.as_f64());
            let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("success");
            let error = if subtype == "success" { None } else { Some(subtype.to_string()) };
            let _ = app.emit("chat://done", json!({ "cost": cost, "error": error }));
        }
        _ => {}
    }
}

fn emit_block(block: &Value, app: &AppHandle) {
    let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match btype {
        "text" => {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                let _ = app.emit("chat://assistant-text", json!({ "text": text }));
            }
        }
        "tool_use" => {
            let id = block.get("id").and_then(|s| s.as_str()).unwrap_or("");
            let name = block.get("name").and_then(|s| s.as_str()).unwrap_or("?");
            let input = block.get("input").cloned().unwrap_or(Value::Null);
            let _ = app.emit("chat://tool-use", json!({ "id": id, "name": name, "input": input }));
        }
        "thinking" => {
            if let Some(text) = block.get("thinking").and_then(|t| t.as_str()) {
                let _ = app.emit("chat://thinking", json!({ "text": text }));
            }
        }
        _ => {}
    }
}

fn emit_tool_result(block: &Value, app: &AppHandle) {
    let tool_use_id = block.get("tool_use_id").and_then(|t| t.as_str()).unwrap_or("");
    let is_error = block.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
    let summary = summarize_result(block);
    let _ = app.emit("chat://tool-result", json!({
        "toolUseId": tool_use_id,
        "ok": !is_error,
        "summary": summary,
    }));
}

fn summarize_result(block: &Value) -> String {
    let raw = match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items.iter()
            .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    let first_line = raw.lines().next().unwrap_or("");
    if first_line.len() > 200 { format!("{}…", &first_line[..200]) } else { first_line.to_string() }
}

fn ensure_runtime_dir() -> AppResult<PathBuf> {
    let dir = dirs::cache_dir()
        .ok_or_else(|| AppError::Other("no cache dir".into()))?
        .join("agent-console");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn ensure_hook_script(runtime_dir: &Path) -> AppResult<PathBuf> {
    let path = runtime_dir.join("pretool-hook.cjs");
    fs::write(&path, HOOK_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&path)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&path, perm)?;
    }
    Ok(path)
}

fn write_session_settings(runtime_dir: &Path, hook_script: &Path) -> AppResult<PathBuf> {
    let settings = json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": hook_script.to_string_lossy(),
                }]
            }]
        }
    });
    let path = runtime_dir.join("session-settings.json");
    fs::write(&path, settings.to_string())?;
    Ok(path)
}
