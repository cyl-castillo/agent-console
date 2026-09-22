//! Agent Console native hook bridge.
//!
//! One tiny binary, six personalities — `hook-bridge <userprompt|pretooluse|
//! posttooluse|stop|stopfailure|modelswitch>` — replacing the node `.cjs`
//! scripts that made Node a hard requirement of the app (and whose absence made
//! hooks fail silently: the Windows/Melissa class of bug). Behavior and on-disk
//! protocol are byte-compatible with the scripts they replace:
//!
//! - Events append to `<AGENT_CONSOLE_SESSION_DIR>/events.jsonl`, one JSON
//!   object per line, `ts` in epoch millis.
//! - PreToolUse writes `<session>/approvals/<uuid>.req.json`, polls for
//!   `<uuid>.res.json`, cleans both up, and emits either `{}` (defer to the
//!   CLI's native prompt — the shape BOTH engines read as "no decision") or
//!   the shared `hookSpecificOutput.permissionDecision` schema.
//! - UserPromptSubmit POSTs to the app's loopback inject endpoint (port from
//!   `inject-port.json` in the platform data dir) and echoes
//!   `additionalContext` / `sessionTitle` back as `hookSpecificOutput`.
//!
//! Outside Agent Console (env vars unset) every mode is a silent no-op, so a
//! user's regular `claude` / `codex` sessions are unaffected.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

const EXCERPT_MAX: usize = 1000;
const SUMMARY_MAX: usize = 1000;
const DETAILS_MAX: usize = 1000;
/// Model names reach the resume command (`claude --model <m>`), so the reader
/// validates them; this cap only keeps a hostile payload out of events.jsonl.
const MODEL_MAX: usize = 128;
const MIN_PROMPT_CHARS: usize = 12;
const INJECT_TIMEOUT_MS: u64 = 2500;
const APPROVAL_POLL_MS: u64 = 80;
const DEFAULT_APPROVAL_TIMEOUT_MS: u64 = 90_000;
/// Cap on the `permission_suggestions` array forwarded from PermissionRequest.
const MAX_SUGGESTIONS: usize = 16;
/// Cap on a Notification's message text.
const NOTIFICATION_MAX: usize = 500;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// UTF-8-safe prefix cap: the .cjs used JS `slice` (UTF-16 units); here we cap
/// by chars, which is what the Rust readers (`truncate_chars`) also do.
fn cap(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() > max {
        (text.chars().take(max).collect(), true)
    } else {
        (text.to_string(), false)
    }
}

fn str_field(input: &Value, a: &str, b: &str) -> Option<String> {
    for key in [a, b] {
        if let Some(s) = input.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// The session dir every mode is gated on. None ⇒ not inside Agent Console.
fn session_dir() -> Option<PathBuf> {
    let dir = std::env::var("AGENT_CONSOLE_SESSION_DIR").ok()?;
    let p = PathBuf::from(dir);
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

fn append_event(dir: &Path, event: &Value) {
    let line = format!("{event}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("events.jsonl"))
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

fn read_stdin_json() -> Value {
    let mut buf = String::new();
    let _ = std::io::stdin().read_to_string(&mut buf);
    serde_json::from_str(&buf).unwrap_or(Value::Object(Map::new()))
}

// --- posttooluse -----------------------------------------------------------

fn posttooluse_event(input: &Value, term_id: Option<&str>, ts: u64) -> Value {
    let mut e = Map::new();
    e.insert("type".into(), json!("tool_result"));
    e.insert("ts".into(), json!(ts));
    if let Some(tool) = str_field(input, "tool_name", "toolName") {
        e.insert("tool".into(), json!(tool));
    }
    let resp = input
        .get("tool_response")
        .or_else(|| input.get("toolResponse"));
    if let Some(resp) = resp {
        let text = match resp {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let (excerpt, truncated) = cap(&text, EXCERPT_MAX);
        e.insert("excerpt".into(), json!(excerpt));
        e.insert("truncated".into(), json!(truncated));
    }
    if let Some(sid) = str_field(input, "session_id", "sessionId") {
        e.insert("sessionId".into(), json!(sid));
    }
    // Correlation handles: tool_use_id pairs the result with its request;
    // agent_id means "inside a subagent" (absent on the main thread).
    if let Some(id) = str_field(input, "tool_use_id", "toolUseId") {
        e.insert("toolUseId".into(), json!(id));
    }
    if let Some(id) = str_field(input, "agent_id", "agentId") {
        e.insert("agentId".into(), json!(id));
    }
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        e.insert("termId".into(), json!(t));
    }
    if let Some(cwd) = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        e.insert("cwd".into(), json!(cwd));
    }
    Value::Object(e)
}

// --- stop ------------------------------------------------------------------

fn stop_event(input: &Value, term_id: Option<&str>, ts: u64) -> Value {
    let mut e = Map::new();
    e.insert("type".into(), json!("turn_end"));
    e.insert("ts".into(), json!(ts));
    if let Some(sid) = str_field(input, "session_id", "sessionId") {
        e.insert("sessionId".into(), json!(sid));
    }
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        e.insert("termId".into(), json!(t));
    }
    if let Some(cwd) = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        e.insert("cwd".into(), json!(cwd));
    }
    if let Some(last) = str_field(input, "last_assistant_message", "lastAssistantMessage") {
        let trimmed = last.trim();
        if !trimmed.is_empty() {
            let (summary, truncated) = cap(trimmed, SUMMARY_MAX);
            e.insert("summary".into(), json!(summary));
            e.insert("summaryTruncated".into(), json!(truncated));
        }
    }
    Value::Object(e)
}

// --- stopfailure -----------------------------------------------------------

/// StopFailure (Claude 2.1.78+) fires INSTEAD of Stop when the turn ends on an
/// API error, so the turn would otherwise never close. `error` is the CLI's own
/// enum (authentication_failed, rate_limit, billing_error, …) and is passed
/// through verbatim — the app classifies it, and a value we don't know yet must
/// still reach the UI.
fn stopfailure_event(input: &Value, term_id: Option<&str>, ts: u64) -> Value {
    let mut e = Map::new();
    e.insert("type".into(), json!("turn_failed"));
    e.insert("ts".into(), json!(ts));
    if let Some(err) = str_field(input, "error", "errorType") {
        e.insert("error".into(), json!(err));
    }
    if let Some(details) = str_field(input, "error_details", "errorDetails") {
        let trimmed = details.trim();
        if !trimmed.is_empty() {
            let (text, truncated) = cap(trimmed, DETAILS_MAX);
            e.insert("errorDetails".into(), json!(text));
            e.insert("errorDetailsTruncated".into(), json!(truncated));
        }
    }
    if let Some(sid) = str_field(input, "session_id", "sessionId") {
        e.insert("sessionId".into(), json!(sid));
    }
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        e.insert("termId".into(), json!(t));
    }
    if let Some(cwd) = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        e.insert("cwd".into(), json!(cwd));
    }
    if let Some(last) = str_field(input, "last_assistant_message", "lastAssistantMessage") {
        let trimmed = last.trim();
        if !trimmed.is_empty() {
            let (summary, truncated) = cap(trimmed, SUMMARY_MAX);
            e.insert("summary".into(), json!(summary));
            e.insert("summaryTruncated".into(), json!(truncated));
        }
    }
    Value::Object(e)
}

// --- modelswitch -----------------------------------------------------------

/// PostModelSwitch → `model_switch`. None ⇒ nothing worth recording: a subagent's
/// own switch (it says nothing about the session the pill describes) or a payload
/// without a destination model.
fn modelswitch_event(input: &Value, term_id: Option<&str>, ts: u64) -> Option<Value> {
    // `agent_id` is only present inside a subagent, so its presence IS the
    // filter — without it a Task run would rewrite the session's model.
    if str_field(input, "agent_id", "agentId").is_some() {
        return None;
    }
    let to = str_field(input, "to_model", "toModel")?;
    let to = to.trim();
    if to.is_empty() {
        return None;
    }
    let mut e = Map::new();
    e.insert("type".into(), json!("model_switch"));
    e.insert("ts".into(), json!(ts));
    e.insert("model".into(), json!(cap(to, MODEL_MAX).0));
    if let Some(from) = str_field(input, "from_model", "fromModel") {
        let from = from.trim();
        if !from.is_empty() {
            e.insert("fromModel".into(), json!(cap(from, MODEL_MAX).0));
        }
    }
    if let Some(sid) = str_field(input, "session_id", "sessionId") {
        e.insert("sessionId".into(), json!(sid));
    }
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        e.insert("termId".into(), json!(t));
    }
    Some(Value::Object(e))
}

// --- userprompt ------------------------------------------------------------

fn userprompt_event(input: &Value, term_id: Option<&str>, ts: u64) -> Value {
    let prompt = str_field(input, "user_prompt", "prompt")
        .or_else(|| str_field(input, "message", "message"))
        .unwrap_or_default();
    let mut e = Map::new();
    e.insert("type".into(), json!("user_prompt"));
    e.insert("ts".into(), json!(ts));
    e.insert("prompt".into(), json!(prompt));
    if let Some(sid) = str_field(input, "session_id", "sessionId") {
        e.insert("sessionId".into(), json!(sid));
    }
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        e.insert("termId".into(), json!(t));
    }
    if let Some(cwd) = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        e.insert("cwd".into(), json!(cwd));
    }
    // Leading slash command — likely a skill or custom command invocation.
    if let Some(rest) = prompt.strip_prefix('/') {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .collect();
        if !name.is_empty() {
            e.insert("skill".into(), json!(name));
        }
    }
    Value::Object(e)
}

/// Mirrors the Rust app side's `dirs::data_local_dir()` — where
/// `inject-port.json` lives (same resolution the .cjs used).
fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Ok(x) = std::env::var("XDG_DATA_HOME") {
            if !x.is_empty() {
                return Some(PathBuf::from(x));
            }
        }
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".local").join("share"))
    }
}

/// Port + per-process token of the app's loopback endpoint, from the same
/// owner-only file. The app refuses requests without the token, so a file
/// we can't read (or one without a token) means "inject nothing" — never
/// "try anyway".
fn inject_target() -> Option<(u16, String)> {
    let raw =
        std::fs::read_to_string(data_dir()?.join("agent-console").join("inject-port.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let port = v.get("port")?.as_u64()?;
    let port = u16::try_from(port).ok().filter(|p| *p > 0)?;
    let token = v
        .get("token")?
        .as_str()
        .filter(|t| !t.is_empty())?
        .to_string();
    Some((port, token))
}

/// Minimal HTTP/1.1 POST to the loopback inject endpoint — hand-rolled over
/// TcpStream like the server side, so the bridge stays dependency-free. The
/// whole exchange lives inside INJECT_TIMEOUT_MS; any failure returns None
/// (inject nothing — the prompt must never wait on us).
fn fetch_injection(port: u16, token: &str, body: &str, budget: Duration) -> Option<Value> {
    let start = Instant::now();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, budget).ok()?;
    fn remaining(start: Instant, budget: Duration) -> Option<Duration> {
        budget.checked_sub(start.elapsed()).filter(|d| !d.is_zero())
    }
    stream.set_write_timeout(remaining(start, budget)).ok()?;
    let req = format!(
        "POST /inject HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nX-Agent-Console-Token: {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).ok()?;
    stream.set_read_timeout(remaining(start, budget)).ok()?;
    let mut resp = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        remaining(start, budget)?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => resp.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    }
    let text = String::from_utf8_lossy(&resp);
    let body_start = text.find("\r\n\r\n")? + 4;
    serde_json::from_str(&text[body_start..]).ok()
}

fn run_userprompt(dir: &Path) {
    let input = read_stdin_json();
    let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
    let event = userprompt_event(&input, term_id.as_deref(), now_ms());
    append_event(dir, &event);

    let prompt = event.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    // Slash commands carry their own instructions; short prompts carry nothing
    // to search for. Both skip straight to a clean exit.
    if prompt.chars().count() < MIN_PROMPT_CHARS || prompt.starts_with('/') {
        return;
    }
    let Some((port, token)) = inject_target() else {
        return;
    };
    let body = json!({
        "prompt": prompt,
        "cwd": event.get("cwd").and_then(|v| v.as_str()).unwrap_or(""),
        "termId": term_id.as_deref().filter(|t| !t.is_empty()),
    })
    .to_string();
    let Some(answer) = fetch_injection(
        port,
        &token,
        &body,
        Duration::from_millis(INJECT_TIMEOUT_MS),
    ) else {
        return;
    };
    let mut out = Map::new();
    out.insert("hookEventName".into(), json!("UserPromptSubmit"));
    if let Some(ctx) = answer
        .get("context")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        out.insert("additionalContext".into(), json!(ctx));
    }
    if let Some(title) = answer
        .get("sessionTitle")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        out.insert("sessionTitle".into(), json!(title));
    }
    // Nothing to say → say nothing at all.
    if out.len() > 1 {
        print!("{}", json!({ "hookSpecificOutput": Value::Object(out) }));
    }
}

// --- pretooluse ------------------------------------------------------------

fn approval_timeout_ms() -> u64 {
    std::env::var("AGENT_CONSOLE_APPROVAL_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_APPROVAL_TIMEOUT_MS)
}

fn pretooluse_request(
    input: &Value,
    session_dir: &Path,
    term_id: Option<&str>,
    id: &str,
    ts: u64,
    timeout_ms: u64,
) -> Value {
    let mut req = Map::new();
    req.insert("id".into(), json!(id));
    req.insert("ts".into(), json!(ts));
    req.insert("sessionDir".into(), json!(session_dir.to_string_lossy()));
    let cwd = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        })
        .unwrap_or_default();
    req.insert("cwd".into(), json!(cwd));
    req.insert(
        "tool".into(),
        json!(str_field(input, "tool_name", "toolName").unwrap_or_else(|| "Unknown".into())),
    );
    req.insert(
        "input".into(),
        input
            .get("tool_input")
            .cloned()
            .unwrap_or(Value::Object(Map::new())),
    );
    req.insert("timeoutMs".into(), json!(timeout_ms));
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        req.insert("termId".into(), json!(t));
    }
    Value::Object(req)
}

/// PermissionRequest (Claude 2.1.x) carries what PreToolUse does — minus
/// `tool_use_id` — plus `permission_mode` and the CLI's own
/// `permission_suggestions` (the "always allow" rules it would offer). The
/// request is tagged `source` so the ledger and the modal can tell the two
/// bridges apart: this one only fires when Claude was about to ASK, so every
/// request here is a real decision point, never a tool the rules already
/// allowed.
fn permissionrequest_request(
    input: &Value,
    session_dir: &Path,
    term_id: Option<&str>,
    id: &str,
    ts: u64,
    timeout_ms: u64,
) -> Value {
    let mut req = pretooluse_request(input, session_dir, term_id, id, ts, timeout_ms);
    let obj = req
        .as_object_mut()
        .expect("pretooluse_request builds an object");
    obj.insert("source".into(), json!("permission_request"));
    if let Some(m) = input.get("permission_mode").and_then(|v| v.as_str()) {
        obj.insert("permissionMode".into(), json!(m));
    }
    if let Some(s) = input
        .get("permission_suggestions")
        .and_then(|v| v.as_array())
    {
        // Bounded: the array is the CLI's, but it ends up in a JSON file the
        // UI reads and the ledger stores.
        if s.len() <= MAX_SUGGESTIONS {
            obj.insert("permissionSuggestions".into(), Value::Array(s.clone()));
        }
    }
    if let Some(sid) = str_field(input, "session_id", "sessionId") {
        obj.insert("sessionId".into(), json!(sid));
    }
    req
}

/// PermissionRequest's decision schema differs from PreToolUse's: a nested
/// `decision` object with `behavior`, `message` (deny) — and exit code 2 is
/// NOT honored for this event, so the object is the only lever. None / ask
/// ⇒ `{}`: the CLI shows its own prompt (interactive) or, where it can't
/// prompt, auto-denies — its documented default, not ours.
fn permission_decision_output(decision: Option<&Value>) -> String {
    let Some(res) = decision else {
        return "{}".into();
    };
    let d = res.get("decision").and_then(|v| v.as_str()).unwrap_or("");
    let reason = res
        .get("reason")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let decision = match d {
        "allow" => json!({ "behavior": "allow" }),
        "deny" => json!({
            "behavior": "deny",
            "message": reason.unwrap_or_else(|| "denied in the Agent Console approval modal".into()),
        }),
        _ => return "{}".into(),
    };
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": decision,
        }
    })
    .to_string()
}

/// The console did NOT answer this request in time (timeout, or an explicit
/// "ask"): the decision is about to be made outside it — in the CLI's own
/// prompt, or by its auto-deny where it can't prompt. Recorded so the ledger
/// never shows a request with no outcome; until now 58 % of approval
/// requests ended exactly like that.
fn approval_deferred_event(id: &str, tool: Option<&str>, term_id: Option<&str>, ts: u64) -> Value {
    let mut e = Map::new();
    e.insert("type".into(), json!("approval_deferred"));
    e.insert("ts".into(), json!(ts));
    e.insert("approvalId".into(), json!(id));
    if let Some(t) = tool {
        e.insert("tool".into(), json!(t));
    }
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        e.insert("termId".into(), json!(t));
    }
    Value::Object(e)
}

fn is_decided(decision: Option<&Value>) -> bool {
    matches!(
        decision
            .and_then(|d| d.get("decision"))
            .and_then(|v| v.as_str()),
        Some("allow") | Some("deny")
    )
}

/// Notification (Claude): `permission_prompt`, `idle_prompt`,
/// `agent_needs_input`, `agent_completed`, … — the CLI saying what it is
/// waiting for. Observer only; the CLI ignores our output.
fn notification_event(input: &Value, term_id: Option<&str>, ts: u64) -> Value {
    let mut e = Map::new();
    e.insert("type".into(), json!("notification"));
    e.insert("ts".into(), json!(ts));
    if let Some(k) = str_field(input, "notification_type", "notificationType") {
        e.insert("notificationType".into(), json!(k));
    }
    if let Some(m) = input.get("message").and_then(|v| v.as_str()) {
        let (m, _) = cap(m, NOTIFICATION_MAX);
        e.insert("message".into(), json!(m));
    }
    if let Some(t) = input.get("title").and_then(|v| v.as_str()) {
        let (t, _) = cap(t, 200);
        e.insert("title".into(), json!(t));
    }
    if let Some(sid) = str_field(input, "session_id", "sessionId") {
        e.insert("sessionId".into(), json!(sid));
    }
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        e.insert("termId".into(), json!(t));
    }
    if let Some(cwd) = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        e.insert("cwd".into(), json!(cwd));
    }
    Value::Object(e)
}

/// The decision output for stdout. None ⇒ emit `{}` (defer to the CLI's own
/// prompt); the empty object means "no decision" to BOTH engines.
fn decision_output(decision: Option<&Value>) -> String {
    let Some(res) = decision else {
        return "{}".into();
    };
    let d = res.get("decision").and_then(|v| v.as_str()).unwrap_or("");
    if d != "allow" && d != "deny" {
        return "{}".into();
    }
    let reason = res
        .get("reason")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("agent-console approval modal: {d}"));
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": d,
            "permissionDecisionReason": reason,
        }
    })
    .to_string()
}

/// Shared half of both approval bridges: write the request where the app's
/// watcher sees it, poll for the answer until the deadline, clean up. None ⇒
/// no decision arrived in time.
fn await_decision(session_dir: &Path, id: &str, req: &Value, timeout_ms: u64) -> Option<Value> {
    let approvals = session_dir.join("approvals");
    let _ = std::fs::create_dir_all(&approvals);
    let req_path = approvals.join(format!("{id}.req.json"));
    let res_path = approvals.join(format!("{id}.res.json"));
    if std::fs::write(&req_path, req.to_string()).is_err() {
        // If we can't write the request, fail open to the CLI's native prompt.
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut decision: Option<Value> = None;
    while Instant::now() < deadline {
        if res_path.exists() {
            // The file may still be mid-write; keep polling on parse failure.
            if let Ok(txt) = std::fs::read_to_string(&res_path) {
                if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                    decision = Some(v);
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(APPROVAL_POLL_MS));
    }
    let _ = std::fs::remove_file(&req_path);
    let _ = std::fs::remove_file(&res_path);
    decision
}

/// Both bridges share the same shape of "what happens when the console
/// doesn't answer": leave a `approval_deferred` line so the ledger closes the
/// request as decided-outside, then defer to the CLI.
fn record_if_deferred(
    session_dir: &Path,
    decision: Option<&Value>,
    id: &str,
    req: &Value,
    term_id: Option<&str>,
) {
    if !is_decided(decision) {
        append_event(
            session_dir,
            &approval_deferred_event(
                id,
                req.get("tool").and_then(|v| v.as_str()),
                term_id,
                now_ms(),
            ),
        );
    }
}

/// Legacy approvals bridge (PreToolUse): fires on EVERY tool call. Kept for
/// Codex, whose hooks table mirrors Claude's but has no PermissionRequest.
fn run_pretooluse(session_dir: &Path) {
    if std::env::var("AGENT_CONSOLE_BRIDGE").as_deref() != Ok("1") {
        return;
    }
    let input = read_stdin_json();
    let id = uuid::Uuid::new_v4().to_string();
    let timeout_ms = approval_timeout_ms();
    let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
    let req = pretooluse_request(
        &input,
        session_dir,
        term_id.as_deref(),
        &id,
        now_ms(),
        timeout_ms,
    );
    let decision = await_decision(session_dir, &id, &req, timeout_ms);
    record_if_deferred(
        session_dir,
        decision.as_ref(),
        &id,
        &req,
        term_id.as_deref(),
    );
    print!("{}", decision_output(decision.as_ref()));
}

/// Approvals bridge, second generation (PermissionRequest, Claude): fires
/// only when the CLI was about to ask the human — tools its rules already
/// allow never reach the modal and never spawn this process.
fn run_permissionrequest(session_dir: &Path) {
    if std::env::var("AGENT_CONSOLE_BRIDGE").as_deref() != Ok("1") {
        return;
    }
    let input = read_stdin_json();
    let id = uuid::Uuid::new_v4().to_string();
    let timeout_ms = approval_timeout_ms();
    let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
    let req = permissionrequest_request(
        &input,
        session_dir,
        term_id.as_deref(),
        &id,
        now_ms(),
        timeout_ms,
    );
    let decision = await_decision(session_dir, &id, &req, timeout_ms);
    record_if_deferred(
        session_dir,
        decision.as_ref(),
        &id,
        &req,
        term_id.as_deref(),
    );
    print!("{}", permission_decision_output(decision.as_ref()));
}

// --- statusline ------------------------------------------------------------

/// Where the user's ORIGINAL statusLine setting is parked while ours is
/// installed (same data dir as inject-port.json). `{"command": …, "padding": …}`
/// or `{}` when they had none.
fn statusline_chain_path() -> Option<PathBuf> {
    Some(
        data_dir()?
            .join("agent-console")
            .join("statusline-chain.json"),
    )
}

/// The console's view of one status-line render: model, cost, context — the
/// numbers the CLI computes itself, which until now the app re-derived from
/// the transcript every 5 s (and guessed the window size). Input-only context
/// sum matches `used_percentage`'s own formula.
fn status_event(input: &Value, term_id: Option<&str>, ts: u64) -> Value {
    let mut e = Map::new();
    e.insert("type".into(), json!("status"));
    e.insert("ts".into(), json!(ts));
    if let Some(sid) = str_field(input, "session_id", "sessionId") {
        e.insert("sessionId".into(), json!(sid));
    }
    if let Some(t) = term_id.filter(|t| !t.is_empty()) {
        e.insert("termId".into(), json!(t));
    }
    if let Some(m) = input.get("model") {
        if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
            e.insert("modelId".into(), json!(id));
        }
        if let Some(n) = m.get("display_name").and_then(|v| v.as_str()) {
            e.insert("modelName".into(), json!(n));
        }
    }
    if let Some(c) = input.get("cost") {
        if let Some(v) = c.get("total_cost_usd").and_then(|v| v.as_f64()) {
            e.insert("costUsd".into(), json!(v));
        }
        if let Some(v) = c.get("total_lines_added").and_then(|v| v.as_u64()) {
            e.insert("linesAdded".into(), json!(v));
        }
        if let Some(v) = c.get("total_lines_removed").and_then(|v| v.as_u64()) {
            e.insert("linesRemoved".into(), json!(v));
        }
        if let Some(v) = c.get("total_duration_ms").and_then(|v| v.as_u64()) {
            e.insert("durationMs".into(), json!(v));
        }
    }
    if let Some(cw) = input.get("context_window") {
        if let Some(v) = cw.get("context_window_size").and_then(|v| v.as_u64()) {
            e.insert("contextSize".into(), json!(v));
        }
        if let Some(v) = cw.get("used_percentage").and_then(|v| v.as_f64()) {
            e.insert("usedPct".into(), json!(v));
        }
        if let Some(u) = cw.get("current_usage") {
            let n = |k: &str| u.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
            let used =
                n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
            e.insert("contextUsed".into(), json!(used));
            e.insert("outputTokens".into(), json!(n("output_tokens")));
        }
        if let Some(v) = cw.get("total_input_tokens").and_then(|v| v.as_u64()) {
            e.insert("inputTotal".into(), json!(v));
        }
        if let Some(v) = cw.get("total_output_tokens").and_then(|v| v.as_u64()) {
            e.insert("outputTotal".into(), json!(v));
        }
    }
    if let Some(b) = input.get("exceeds_200k_tokens").and_then(|v| v.as_bool()) {
        e.insert("exceeds200k".into(), json!(b));
    }
    Value::Object(e)
}

/// Everything but `ts`: the fingerprint that decides whether this render
/// changed anything worth a new line in events.jsonl (the status line
/// re-renders on vim-mode toggles and the like, which change nothing here).
fn status_fingerprint(event: &Value) -> String {
    let mut m = event.as_object().cloned().unwrap_or_default();
    m.remove("ts");
    Value::Object(m).to_string()
}

/// Run the user's original status line command with the same stdin and pass
/// its output through. No chain ⇒ print nothing (they had no status line,
/// they still don't). Never lets a failure reach the CLI: an error here is
/// an empty status line, not a broken session.
fn chain_statusline(raw_input: &[u8]) {
    let Some(chain) = statusline_chain_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return;
    };
    let Some(cmd) = chain
        .get("command")
        .and_then(|v| v.as_str())
        .filter(|c| !c.trim().is_empty())
    else {
        return;
    };
    #[cfg(windows)]
    let mut child = std::process::Command::new("cmd");
    #[cfg(windows)]
    child.args(["/C", cmd]);
    #[cfg(not(windows))]
    let mut child = std::process::Command::new("/bin/sh");
    #[cfg(not(windows))]
    child.args(["-c", cmd]);
    let Ok(mut child) = child
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(raw_input);
    }
    if let Ok(out) = child.wait_with_output() {
        let _ = std::io::stdout().write_all(&out.stdout);
    }
}

/// statusLine mode: runs on EVERY render, inside and outside the console —
/// so unlike the hooks it must not gate on the session dir. Inside: record
/// the render (deduped) for the app. Always: chain to the user's own line.
fn run_statusline() {
    let mut raw = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut raw);
    if let Some(dir) = session_dir() {
        let input: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
        let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
        let event = status_event(&input, term_id.as_deref(), now_ms());
        let fp = status_fingerprint(&event);
        let last_path = dir.join(format!(
            "status-last-{}.txt",
            term_id.as_deref().unwrap_or("default")
        ));
        let unchanged = std::fs::read_to_string(&last_path)
            .map(|prev| prev == fp)
            .unwrap_or(false);
        if !unchanged {
            append_event(&dir, &event);
            let _ = std::fs::write(&last_path, fp);
        }
    }
    chain_statusline(&raw);
}

// --- main ------------------------------------------------------------------

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    // The status line is the one mode that must run OUTSIDE the console too:
    // it stands in for the user's own status line command and chains to it.
    if mode == "statusline" {
        run_statusline();
        return;
    }
    // Every other mode is a silent no-op outside Agent Console.
    let Some(dir) = session_dir() else { return };
    match mode.as_str() {
        "userprompt" => run_userprompt(&dir),
        "pretooluse" => run_pretooluse(&dir),
        "permissionrequest" => run_permissionrequest(&dir),
        "notification" => {
            let input = read_stdin_json();
            let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
            append_event(
                &dir,
                &notification_event(&input, term_id.as_deref(), now_ms()),
            );
        }
        "posttooluse" => {
            let input = read_stdin_json();
            let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
            append_event(
                &dir,
                &posttooluse_event(&input, term_id.as_deref(), now_ms()),
            );
        }
        "stop" => {
            let input = read_stdin_json();
            let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
            append_event(&dir, &stop_event(&input, term_id.as_deref(), now_ms()));
        }
        "stopfailure" => {
            let input = read_stdin_json();
            let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
            append_event(
                &dir,
                &stopfailure_event(&input, term_id.as_deref(), now_ms()),
            );
        }
        "modelswitch" => {
            let input = read_stdin_json();
            let term_id = std::env::var("AGENT_CONSOLE_TERM_ID").ok();
            if let Some(e) = modelswitch_event(&input, term_id.as_deref(), now_ms()) {
                append_event(&dir, &e);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_event_caps_the_excerpt_and_keeps_attribution() {
        let input = json!({
            "tool_name": "Bash",
            "tool_response": "x".repeat(EXCERPT_MAX + 50),
            "session_id": "s1",
            "cwd": "/repo",
        });
        let e = posttooluse_event(&input, Some("t-1"), 42);
        assert_eq!(e["type"], "tool_result");
        assert_eq!(e["ts"], 42);
        assert_eq!(e["tool"], "Bash");
        assert_eq!(e["excerpt"].as_str().unwrap().len(), EXCERPT_MAX);
        assert_eq!(e["truncated"], true);
        assert_eq!(e["sessionId"], "s1");
        assert_eq!(e["termId"], "t-1");
        assert_eq!(e["cwd"], "/repo");
    }

    #[test]
    fn turn_end_carries_the_agents_words_when_present_and_nothing_otherwise() {
        let with = stop_event(
            &json!({"last_assistant_message": "  did the thing  "}),
            None,
            1,
        );
        assert_eq!(with["summary"], "did the thing");
        assert_eq!(with["summaryTruncated"], false);
        let without = stop_event(&json!({"session_id": "s"}), None, 1);
        assert!(without.get("summary").is_none());
        assert!(without.get("summaryTruncated").is_none());
    }

    #[test]
    fn model_switch_records_the_destination_and_binds_it_to_the_terminal() {
        let e = modelswitch_event(
            &json!({
                "from_model": "claude-sonnet-5",
                "to_model": "  claude-opus-5  ",
                "session_id": "s1",
            }),
            Some("t-1"),
            9,
        )
        .expect("a real session switch is recorded");
        assert_eq!(e["type"], "model_switch");
        assert_eq!(e["ts"], 9);
        assert_eq!(e["model"], "claude-opus-5");
        assert_eq!(e["fromModel"], "claude-sonnet-5");
        assert_eq!(e["sessionId"], "s1");
        assert_eq!(e["termId"], "t-1");
    }

    /// A subagent switching its own model must not rewrite the session's pill,
    /// and a payload with no destination has nothing to report.
    #[test]
    fn model_switch_ignores_subagents_and_empty_destinations() {
        assert!(modelswitch_event(
            &json!({"to_model": "claude-haiku-4-5", "agent_id": "sub-1"}),
            Some("t-1"),
            1
        )
        .is_none());
        assert!(modelswitch_event(&json!({"to_model": "   "}), None, 1).is_none());
        assert!(modelswitch_event(&json!({"from_model": "claude-opus-5"}), None, 1).is_none());
    }

    #[test]
    fn model_switch_caps_an_absurd_model_name() {
        let e = modelswitch_event(&json!({"to_model": "x".repeat(MODEL_MAX + 50)}), None, 1)
            .expect("still recorded, just bounded");
        assert_eq!(e["model"].as_str().unwrap().len(), MODEL_MAX);
        assert!(e.get("fromModel").is_none());
    }

    #[test]
    fn turn_failed_carries_the_reason_and_caps_the_details() {
        let e = stopfailure_event(
            &json!({
                "error": "authentication_failed",
                "error_details": "x".repeat(DETAILS_MAX + 50),
                "session_id": "s1",
                "cwd": "/repo",
            }),
            Some("t-1"),
            42,
        );
        assert_eq!(e["type"], "turn_failed");
        assert_eq!(e["ts"], 42);
        assert_eq!(e["error"], "authentication_failed");
        assert_eq!(e["errorDetails"].as_str().unwrap().len(), DETAILS_MAX);
        assert_eq!(e["errorDetailsTruncated"], true);
        assert_eq!(e["sessionId"], "s1");
        assert_eq!(e["termId"], "t-1");
        assert_eq!(e["cwd"], "/repo");
    }

    #[test]
    fn turn_failed_passes_unknown_error_kinds_through_and_omits_absent_fields() {
        // A reason enum added by a future CLI must still reach the UI.
        let e = stopfailure_event(&json!({"error": "some_future_kind"}), None, 1);
        assert_eq!(e["error"], "some_future_kind");
        assert!(e.get("errorDetails").is_none());
        assert!(e.get("summary").is_none());
        // A payload without a reason at all is still a valid close.
        let bare = stopfailure_event(&json!({}), None, 1);
        assert_eq!(bare["type"], "turn_failed");
        assert!(bare.get("error").is_none());
    }

    #[test]
    fn user_prompt_event_detects_slash_skills() {
        let e = userprompt_event(&json!({"prompt": "/review-pr the thing"}), None, 1);
        assert_eq!(e["skill"], "review-pr");
        let plain = userprompt_event(&json!({"prompt": "hello world"}), None, 1);
        assert!(plain.get("skill").is_none());
    }

    #[test]
    fn posttooluse_event_carries_correlation_ids_when_present() {
        let e = posttooluse_event(
            &json!({
                "tool_name": "Bash", "tool_response": "ok", "session_id": "s",
                "tool_use_id": "toolu_1", "agent_id": "sub-1"
            }),
            Some("t"),
            1,
        );
        assert_eq!(e["toolUseId"], "toolu_1");
        assert_eq!(e["agentId"], "sub-1");
        let bare = posttooluse_event(&json!({"tool_name": "Read", "tool_response": "x"}), None, 1);
        assert!(bare.get("toolUseId").is_none());
        assert!(bare.get("agentId").is_none());
    }

    #[test]
    fn pretooluse_request_matches_the_cjs_shape() {
        let input = json!({
            "tool_name": "Bash",
            "tool_input": {"command": "git push"},
            "cwd": "/repo",
        });
        let req = pretooluse_request(&input, Path::new("/sess"), Some("t-9"), "abc", 7, 90_000);
        assert_eq!(req["id"], "abc");
        assert_eq!(req["ts"], 7);
        assert_eq!(req["sessionDir"], "/sess");
        assert_eq!(req["cwd"], "/repo");
        assert_eq!(req["tool"], "Bash");
        assert_eq!(req["input"]["command"], "git push");
        assert_eq!(req["timeoutMs"], 90_000);
        assert_eq!(req["termId"], "t-9");
    }

    #[test]
    fn permissionrequest_request_adds_source_mode_and_suggestions() {
        let input = json!({
            "session_id": "s-1",
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf node_modules"},
            "cwd": "/repo",
            "permission_mode": "default",
            "permission_suggestions": [
                {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "rm -rf node_modules"}],
                 "behavior": "allow", "destination": "localSettings"}
            ],
        });
        let req =
            permissionrequest_request(&input, Path::new("/sess"), Some("t-9"), "abc", 7, 90_000);
        // Everything PreToolUse carried…
        assert_eq!(req["tool"], "Bash");
        assert_eq!(req["input"]["command"], "rm -rf node_modules");
        assert_eq!(req["termId"], "t-9");
        assert_eq!(req["timeoutMs"], 90_000);
        // …plus what makes this a real decision point.
        assert_eq!(req["source"], "permission_request");
        assert_eq!(req["permissionMode"], "default");
        assert_eq!(req["sessionId"], "s-1");
        assert_eq!(req["permissionSuggestions"][0]["type"], "addRules");
        // Absent fields stay absent (older CLI payloads).
        let bare = permissionrequest_request(
            &json!({"tool_name": "Read", "tool_input": {}}),
            Path::new("/sess"),
            None,
            "x",
            1,
            10,
        );
        assert!(bare.get("permissionSuggestions").is_none());
        assert!(bare.get("permissionMode").is_none());
        assert!(bare.get("termId").is_none());
    }

    #[test]
    fn permission_decision_output_uses_the_nested_decision_object() {
        assert_eq!(permission_decision_output(None), "{}");
        assert_eq!(
            permission_decision_output(Some(&json!({"decision": "ask"}))),
            "{}"
        );
        let allow = permission_decision_output(Some(&json!({"decision": "allow"})));
        let v: Value = serde_json::from_str(&allow).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert!(v["hookSpecificOutput"]["decision"].get("message").is_none());
        let deny = permission_decision_output(Some(&json!({"decision": "deny", "reason": "nope"})));
        let v: Value = serde_json::from_str(&deny).unwrap();
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(v["hookSpecificOutput"]["decision"]["message"], "nope");
        let deny_default = permission_decision_output(Some(&json!({"decision": "deny"})));
        assert!(deny_default.contains("approval modal"));
    }

    #[test]
    fn deferral_is_recorded_only_when_no_decision_reached_the_hook() {
        assert!(is_decided(Some(&json!({"decision": "allow"}))));
        assert!(is_decided(Some(&json!({"decision": "deny"}))));
        assert!(!is_decided(Some(&json!({"decision": "ask"}))));
        assert!(!is_decided(None));
        let e = approval_deferred_event("abc", Some("Bash"), Some("t-1"), 42);
        assert_eq!(e["type"], "approval_deferred");
        assert_eq!(e["approvalId"], "abc");
        assert_eq!(e["tool"], "Bash");
        assert_eq!(e["termId"], "t-1");
        assert_eq!(e["ts"], 42);
    }

    #[test]
    fn notification_event_carries_type_message_and_binding() {
        let input = json!({
            "session_id": "s-1",
            "cwd": "/repo",
            "hook_event_name": "Notification",
            "message": "Claude needs your permission",
            "title": "Permission needed",
            "notification_type": "permission_prompt",
        });
        let e = notification_event(&input, Some("t-2"), 9);
        assert_eq!(e["type"], "notification");
        assert_eq!(e["notificationType"], "permission_prompt");
        assert_eq!(e["message"], "Claude needs your permission");
        assert_eq!(e["title"], "Permission needed");
        assert_eq!(e["sessionId"], "s-1");
        assert_eq!(e["termId"], "t-2");
        assert_eq!(e["cwd"], "/repo");
        assert_eq!(e["ts"], 9);
        // A message beyond the cap is cut, never dropped.
        let long = notification_event(&json!({"message": "x".repeat(2000)}), None, 1);
        assert!(long["message"].as_str().unwrap().chars().count() <= NOTIFICATION_MAX + 1);
    }

    #[test]
    fn status_event_extracts_model_cost_and_input_only_context() {
        let input = json!({
            "session_id": "s-1",
            "model": {"id": "claude-opus-5-5", "display_name": "Opus"},
            "cost": {"total_cost_usd": 0.01234, "total_lines_added": 156, "total_lines_removed": 23, "total_duration_ms": 45000},
            "context_window": {
                "total_input_tokens": 15500, "total_output_tokens": 1200,
                "context_window_size": 200000, "used_percentage": 8,
                "current_usage": {"input_tokens": 8500, "output_tokens": 1200,
                                  "cache_creation_input_tokens": 5000, "cache_read_input_tokens": 2000}
            },
            "exceeds_200k_tokens": false
        });
        let e = status_event(&input, Some("t-1"), 5);
        assert_eq!(e["type"], "status");
        assert_eq!(e["sessionId"], "s-1");
        assert_eq!(e["termId"], "t-1");
        assert_eq!(e["modelId"], "claude-opus-5-5");
        assert_eq!(e["modelName"], "Opus");
        assert_eq!(e["costUsd"], 0.01234);
        assert_eq!(e["linesAdded"], 156);
        assert_eq!(e["contextSize"], 200000);
        assert_eq!(e["usedPct"], 8.0);
        // input + cache_creation + cache_read, never output.
        assert_eq!(e["contextUsed"], 15500);
        assert_eq!(e["outputTokens"], 1200);
        assert_eq!(e["exceeds200k"], false);
        // Sparse payload (early in a session): fields simply absent.
        let bare = status_event(&json!({"session_id": "s"}), None, 1);
        assert!(bare.get("contextUsed").is_none());
        assert!(bare.get("costUsd").is_none());
        assert!(bare.get("termId").is_none());
    }

    #[test]
    fn status_fingerprint_ignores_the_timestamp_only() {
        let a = status_event(&json!({"cost": {"total_cost_usd": 1.0}}), Some("t"), 1);
        let b = status_event(&json!({"cost": {"total_cost_usd": 1.0}}), Some("t"), 2);
        let c = status_event(&json!({"cost": {"total_cost_usd": 1.5}}), Some("t"), 2);
        assert_eq!(status_fingerprint(&a), status_fingerprint(&b));
        assert_ne!(status_fingerprint(&b), status_fingerprint(&c));
    }

    #[test]
    fn decision_output_defers_on_timeout_ask_or_garbage() {
        assert_eq!(decision_output(None), "{}");
        assert_eq!(decision_output(Some(&json!({"decision": "ask"}))), "{}");
        assert_eq!(decision_output(Some(&json!({"nonsense": true}))), "{}");
        let out = decision_output(Some(&json!({"decision": "allow", "reason": "ok"})));
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(v["hookSpecificOutput"]["permissionDecisionReason"], "ok");
        let deny = decision_output(Some(&json!({"decision": "deny"})));
        let v: Value = serde_json::from_str(&deny).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["permissionDecisionReason"],
            "agent-console approval modal: deny"
        );
    }

    #[test]
    fn fetch_injection_round_trips_against_a_real_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            // The shared secret must travel as a header, verbatim.
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains("X-Agent-Console-Token: sekrit\r\n"),
                "token header missing in: {req}"
            );
            assert!(req.contains("Content-Type: application/json\r\n"));
            let body = r#"{"context":"remembered","sessionTitle":"my session"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(resp.as_bytes()).unwrap();
        });
        let answer = fetch_injection(port, "sekrit", "{}", Duration::from_millis(2000)).unwrap();
        assert_eq!(answer["context"], "remembered");
        assert_eq!(answer["sessionTitle"], "my session");
        server.join().unwrap();
    }

    #[test]
    fn fetch_injection_gives_up_inside_its_budget_when_the_server_stalls() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Server accepts and then says nothing.
        let server = std::thread::spawn(move || {
            let (_s, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(600));
        });
        let start = Instant::now();
        let answer = fetch_injection(port, "sekrit", "{}", Duration::from_millis(200));
        assert!(answer.is_none());
        assert!(start.elapsed() < Duration::from_millis(550));
        server.join().unwrap();
    }
}
