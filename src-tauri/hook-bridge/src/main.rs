//! Agent Console native hook bridge.
//!
//! One tiny binary, five personalities — `hook-bridge <userprompt|pretooluse|
//! posttooluse|stop|modelswitch>` — replacing the node `.cjs` scripts that made Node a
//! hard requirement of the app (and whose absence made hooks fail silently:
//! the Windows/Melissa class of bug). Behavior and on-disk protocol are
//! byte-compatible with the scripts they replace:
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
/// Model names reach the resume command (`claude --model <m>`), so the reader
/// validates them; this cap only keeps a hostile payload out of events.jsonl.
const MODEL_MAX: usize = 128;
const MIN_PROMPT_CHARS: usize = 12;
const INJECT_TIMEOUT_MS: u64 = 2500;
const APPROVAL_POLL_MS: u64 = 80;
const DEFAULT_APPROVAL_TIMEOUT_MS: u64 = 90_000;

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

fn inject_port() -> Option<u16> {
    let raw =
        std::fs::read_to_string(data_dir()?.join("agent-console").join("inject-port.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let port = v.get("port")?.as_u64()?;
    u16::try_from(port).ok().filter(|p| *p > 0)
}

/// Minimal HTTP/1.1 POST to the loopback inject endpoint — hand-rolled over
/// TcpStream like the server side, so the bridge stays dependency-free. The
/// whole exchange lives inside INJECT_TIMEOUT_MS; any failure returns None
/// (inject nothing — the prompt must never wait on us).
fn fetch_injection(port: u16, body: &str, budget: Duration) -> Option<Value> {
    let start = Instant::now();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, budget).ok()?;
    fn remaining(start: Instant, budget: Duration) -> Option<Duration> {
        budget.checked_sub(start.elapsed()).filter(|d| !d.is_zero())
    }
    stream.set_write_timeout(remaining(start, budget)).ok()?;
    let req = format!(
        "POST /inject HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
    let Some(port) = inject_port() else { return };
    let body = json!({
        "prompt": prompt,
        "cwd": event.get("cwd").and_then(|v| v.as_str()).unwrap_or(""),
        "termId": term_id.as_deref().filter(|t| !t.is_empty()),
    })
    .to_string();
    let Some(answer) = fetch_injection(port, &body, Duration::from_millis(INJECT_TIMEOUT_MS))
    else {
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

fn run_pretooluse(session_dir: &Path) {
    if std::env::var("AGENT_CONSOLE_BRIDGE").as_deref() != Ok("1") {
        return;
    }
    let approvals = session_dir.join("approvals");
    let _ = std::fs::create_dir_all(&approvals);

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
    let req_path = approvals.join(format!("{id}.req.json"));
    let res_path = approvals.join(format!("{id}.res.json"));
    if std::fs::write(&req_path, req.to_string()).is_err() {
        // If we can't write the request, fail open to the CLI's native prompt.
        return;
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
    print!("{}", decision_output(decision.as_ref()));
}

// --- main ------------------------------------------------------------------

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    // Every mode is a silent no-op outside Agent Console.
    let Some(dir) = session_dir() else { return };
    match mode.as_str() {
        "userprompt" => run_userprompt(&dir),
        "pretooluse" => run_pretooluse(&dir),
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
    fn user_prompt_event_detects_slash_skills() {
        let e = userprompt_event(&json!({"prompt": "/review-pr the thing"}), None, 1);
        assert_eq!(e["skill"], "review-pr");
        let plain = userprompt_event(&json!({"prompt": "hello world"}), None, 1);
        assert!(plain.get("skill").is_none());
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
            let _ = s.read(&mut buf);
            let body = r#"{"context":"remembered","sessionTitle":"my session"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(resp.as_bytes()).unwrap();
        });
        let answer = fetch_injection(port, "{}", Duration::from_millis(2000)).unwrap();
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
        let answer = fetch_injection(port, "{}", Duration::from_millis(200));
        assert!(answer.is_none());
        assert!(start.elapsed() < Duration::from_millis(550));
        server.join().unwrap();
    }
}
