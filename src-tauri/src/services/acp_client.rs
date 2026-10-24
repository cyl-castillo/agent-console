//! A minimal Agent Client Protocol client (COMPETENCIA-2026-10 F3a): JSON-RPC
//! 2.0 over the agent's stdio, enough to run one turn and witness it per
//! tool call — `initialize` → `session/new` (or `session/load` to resume) →
//! `session/prompt`, with `session/update` notifications folded into the
//! room's activity feed and `session/request_permission` answered by the
//! turn's `ToolPolicy`. Verified against OpenCode 1.19.3 (`opencode acp`):
//! the fixture in `fixtures/acp/opencode-1.19.3-turn.jsonl` is a real turn.
//!
//! Generic over the streams so the fixture drives the tests without a
//! process; `engine_runner` wires it to a child's stdin/stdout.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::engine_runner::{ActivitySink, ToolPolicy};

const PROTOCOL_VERSION: u64 = 1;
const ID_INIT: u64 = 0;
const ID_SESSION: u64 = 1;
const ID_PROMPT: u64 = 2;
const ID_MODEL: u64 = 3;
const ID_MODE: u64 = 4;
const EXCERPT_MAX: usize = 400;

/// What one turn asks for.
pub struct AcpPrompt<'a> {
    pub cwd: &'a str,
    pub prompt: &'a str,
    /// A session id from a prior turn: `session/load` instead of `session/new`.
    pub resume: Option<&'a str>,
    /// `provider/model` to select through `session/set_config_option`
    /// (ignored by agents that don't support it).
    pub model: Option<&'a str>,
    pub policy: ToolPolicy,
    /// ACP `mcpServers` entries (`{name, command, args, env}`).
    pub mcp_servers: Vec<Value>,
}

/// One witnessed tool call, as the agent reported it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AcpToolCall {
    pub id: String,
    pub title: String,
    /// read · edit · delete · move · search · execute · think · fetch · other
    pub kind: String,
    /// `rawInput.command` when the tool ran a command.
    pub command: Option<String>,
    /// pending · in_progress · completed · failed
    pub status: String,
    /// `rawOutput.metadata.exit` (OpenCode's bash tool) when present.
    pub exit_code: Option<i64>,
    pub output_excerpt: String,
}

#[derive(Debug, Default)]
pub struct AcpTurn {
    pub text: String,
    pub session_id: String,
    /// input + output tokens of the turn (cached reads excluded).
    pub tokens: u64,
    pub cost_usd: f64,
    pub stop_reason: String,
    pub tool_calls: Vec<AcpToolCall>,
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn send<W: Write>(w: &mut W, msg: &Value) -> AppResult<()> {
    w.write_all(msg.to_string().as_bytes())?;
    w.write_all(b"\n")?;
    w.flush()?;
    Ok(())
}

/// Which option answers a permission request under this policy: the most
/// permissive "allow" the agent offers for tools the policy permits, the
/// mildest "reject" otherwise. The policy is the human's standing decision
/// for the turn; nothing here asks.
pub fn permission_answer(policy: ToolPolicy, tool_kind: &str, options: &[Value]) -> Option<String> {
    let allowed = match policy {
        ToolPolicy::Full => true,
        ToolPolicy::AcceptEdits => !matches!(tool_kind, "execute" | "delete" | "move"),
        ToolPolicy::ReadOnly | ToolPolicy::Plan => {
            matches!(tool_kind, "read" | "search" | "fetch" | "think")
        }
    };
    let pick = |kinds: &[&str]| {
        kinds.iter().find_map(|k| {
            options
                .iter()
                .find(|o| o.get("kind").and_then(Value::as_str) == Some(k))
                .and_then(|o| o.get("optionId").and_then(Value::as_str))
                .map(String::from)
        })
    };
    if allowed {
        pick(&["allow_once", "allow_always"])
    } else {
        pick(&["reject_once", "reject_always"])
    }
    .or_else(|| {
        options
            .first()
            .and_then(|o| o.get("optionId").and_then(Value::as_str))
            .map(String::from)
    })
}

fn excerpt(s: &str) -> String {
    if s.chars().count() > EXCERPT_MAX {
        s.chars().take(EXCERPT_MAX).collect()
    } else {
        s.to_string()
    }
}

/// Run one turn over the agent's streams. `on_session` fires as soon as the
/// session id is known (the room binds the connector identity to it).
pub fn run_turn<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    req: &AcpPrompt<'_>,
    on_activity: &ActivitySink,
    on_session: &dyn Fn(&str),
) -> AppResult<AcpTurn> {
    send(
        &mut writer,
        &rpc(
            ID_INIT,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
                "clientInfo": { "name": "agent-console", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
    )?;
    let mut turn = AcpTurn::default();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(AppError::Other(
                "ACP agent closed its stream before finishing the turn".into(),
            ));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let id = msg.get("id").and_then(Value::as_u64);
        let method = msg.get("method").and_then(Value::as_str);
        match (id, method) {
            // --- responses to our requests ---
            (Some(ID_INIT), None) => {
                if let Some(e) = msg.get("error") {
                    return Err(AppError::Other(format!("ACP initialize failed: {e}")));
                }
                let params = match req.resume {
                    Some(sid) => {
                        json!({ "sessionId": sid, "cwd": req.cwd, "mcpServers": req.mcp_servers })
                    }
                    None => json!({ "cwd": req.cwd, "mcpServers": req.mcp_servers }),
                };
                let method = if req.resume.is_some() {
                    "session/load"
                } else {
                    "session/new"
                };
                send(&mut writer, &rpc(ID_SESSION, method, params))?;
            }
            (Some(ID_SESSION), None) => {
                if let Some(e) = msg.get("error") {
                    return Err(AppError::Other(format!("ACP session failed: {e}")));
                }
                let sid = msg
                    .pointer("/result/sessionId")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .or_else(|| req.resume.map(String::from))
                    .ok_or_else(|| AppError::Other("ACP session has no id".into()))?;
                on_session(&sid);
                turn.session_id = sid.clone();
                // Best effort: an agent without these methods just errors
                // back (ignored below) and keeps its defaults.
                if let Some(model) = req.model.filter(|m| !m.is_empty()) {
                    send(
                        &mut writer,
                        &rpc(
                            ID_MODEL,
                            "session/set_config_option",
                            json!({ "sessionId": sid, "configId": "model", "value": model }),
                        ),
                    )?;
                }
                if matches!(req.policy, ToolPolicy::ReadOnly | ToolPolicy::Plan) {
                    send(
                        &mut writer,
                        &rpc(
                            ID_MODE,
                            "session/set_mode",
                            json!({ "sessionId": sid, "modeId": "plan" }),
                        ),
                    )?;
                }
                send(
                    &mut writer,
                    &rpc(
                        ID_PROMPT,
                        "session/prompt",
                        json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": req.prompt }] }),
                    ),
                )?;
            }
            (Some(ID_PROMPT), None) => {
                if let Some(e) = msg.get("error") {
                    return Err(AppError::Other(format!("ACP prompt failed: {e}")));
                }
                let r = msg.get("result").cloned().unwrap_or(Value::Null);
                turn.stop_reason = r
                    .get("stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let n = |k: &str| {
                    r.pointer(&format!("/usage/{k}"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                };
                let from_result = n("inputTokens") + n("outputTokens");
                if from_result > 0 {
                    turn.tokens = from_result;
                }
                return Ok(turn);
            }
            (Some(ID_MODEL), None) | (Some(ID_MODE), None) => {}
            // --- requests from the agent ---
            (Some(rid), Some("session/request_permission")) => {
                let kind = msg
                    .pointer("/params/toolCall/kind")
                    .and_then(Value::as_str)
                    .unwrap_or("other");
                let title = msg
                    .pointer("/params/toolCall/title")
                    .and_then(Value::as_str)
                    .unwrap_or(kind);
                let options = msg
                    .pointer("/params/options")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let answer = permission_answer(req.policy, kind, &options);
                on_activity(
                    "permission",
                    kind,
                    &format!("{title} → {}", answer.as_deref().unwrap_or("cancelled")),
                );
                let result = match answer {
                    Some(opt) => json!({ "outcome": { "outcome": "selected", "optionId": opt } }),
                    None => json!({ "outcome": { "outcome": "cancelled" } }),
                };
                send(
                    &mut writer,
                    &json!({ "jsonrpc": "2.0", "id": rid, "result": result }),
                )?;
            }
            (Some(rid), Some(other)) => {
                // fs/*, terminal/*: we advertised neither; say so.
                send(
                    &mut writer,
                    &json!({ "jsonrpc": "2.0", "id": rid, "error": { "code": -32601, "message": format!("{other} not supported") } }),
                )?;
            }
            // --- notifications ---
            (None, Some("session/update")) => {
                let Some(u) = msg.pointer("/params/update") else {
                    continue;
                };
                fold_update(u, &mut turn, on_activity);
            }
            _ => {}
        }
    }
}

fn fold_update(u: &Value, turn: &mut AcpTurn, on_activity: &ActivitySink) {
    let text_of = |u: &Value| {
        u.pointer("/content/text")
            .and_then(Value::as_str)
            .map(String::from)
    };
    match u.get("sessionUpdate").and_then(Value::as_str) {
        Some("agent_message_chunk") => {
            if let Some(t) = text_of(u) {
                turn.text.push_str(&t);
                if !t.is_empty() {
                    on_activity("text", "", &t);
                }
            }
        }
        Some("agent_thought_chunk") => {
            if let Some(t) = text_of(u) {
                if !t.trim().is_empty() {
                    on_activity("thinking", "", &t);
                }
            }
        }
        Some("tool_call") | Some("tool_call_update") => {
            let Some(id) = u.get("toolCallId").and_then(Value::as_str) else {
                return;
            };
            let idx = match turn.tool_calls.iter().position(|c| c.id == id) {
                Some(i) => i,
                None => {
                    turn.tool_calls.push(AcpToolCall {
                        id: id.to_string(),
                        status: "pending".into(),
                        kind: "other".into(),
                        ..Default::default()
                    });
                    turn.tool_calls.len() - 1
                }
            };
            let call = &mut turn.tool_calls[idx];
            if let Some(t) = u.get("title").and_then(Value::as_str) {
                call.title = t.to_string();
            }
            if let Some(k) = u.get("kind").and_then(Value::as_str) {
                call.kind = k.to_string();
            }
            if let Some(c) = u.pointer("/rawInput/command").and_then(Value::as_str) {
                call.command = Some(c.to_string());
            }
            let prev_status = call.status.clone();
            if let Some(s) = u.get("status").and_then(Value::as_str) {
                call.status = s.to_string();
            }
            if let Some(e) = u
                .pointer("/rawOutput/metadata/exit")
                .and_then(Value::as_i64)
            {
                call.exit_code = Some(e);
            }
            if let Some(out) = u.pointer("/rawOutput/output").and_then(Value::as_str) {
                call.output_excerpt = excerpt(out);
            } else if let Some(c) = u.pointer("/content/0/content/text").and_then(Value::as_str) {
                call.output_excerpt = excerpt(c);
            }
            let label = call.command.clone().unwrap_or_else(|| call.title.clone());
            match call.status.as_str() {
                "completed" | "failed" if prev_status != "completed" && prev_status != "failed" => {
                    let verdict = match (call.exit_code, call.status.as_str()) {
                        (Some(c), _) => format!(" · exit {c}"),
                        (None, "failed") => " · failed".to_string(),
                        _ => String::new(),
                    };
                    on_activity("tool_result", &call.kind, &format!("{label}{verdict}"));
                }
                "in_progress" if prev_status == "pending" => {
                    on_activity("tool", &call.kind, &label);
                }
                _ => {}
            }
        }
        Some("usage_update") => {
            if let Some(c) = u.pointer("/cost/amount").and_then(Value::as_f64) {
                turn.cost_usd = c;
            }
            if turn.tokens == 0 {
                if let Some(used) = u.get("used").and_then(Value::as_u64) {
                    turn.tokens = used;
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const TURN: &str = include_str!("../../fixtures/acp/opencode-1.19.3-turn.jsonl");

    /// The agent's side of the fixture: every `<` line, in order.
    fn agent_lines() -> Vec<u8> {
        TURN.lines()
            .filter_map(|l| l.strip_prefix("< "))
            .map(|l| format!("{l}\n"))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn a_real_opencode_turn_is_replayed_with_its_tool_calls_exit_codes_and_usage() {
        let agent = std::io::Cursor::new(agent_lines());
        let mut sent: Vec<u8> = Vec::new();
        let activity = RefCell::new(Vec::<(String, String, String)>::new());
        let sessions = RefCell::new(Vec::<String>::new());
        let req = AcpPrompt {
            cwd: "/home/u/proj",
            prompt: "Run these two shell commands…",
            resume: None,
            model: Some("opencode/big-pickle"),
            policy: ToolPolicy::Full,
            mcp_servers: vec![
                json!({ "name": "agent_console", "command": "/bridge", "args": ["mcp"], "env": [] }),
            ],
        };
        let turn = run_turn(
            agent,
            &mut sent,
            &req,
            &|k, l, t| activity.borrow_mut().push((k.into(), l.into(), t.into())),
            &|sid| sessions.borrow_mut().push(sid.into()),
        )
        .unwrap();
        assert_eq!(turn.session_id, "ses_e956e2ed1ffe1pcoRVDtgpVjBx");
        assert_eq!(sessions.borrow().as_slice(), &[turn.session_id.clone()]);
        assert_eq!(
            turn.text,
            "The second command failed: `src/b.test.js` — 1 failed, 1 passed. So I can't say all tests pass."
        );
        assert_eq!(turn.stop_reason, "end_turn");
        assert_eq!(
            turn.tokens,
            11330 + 31,
            "input + output, cached reads excluded"
        );
        assert_eq!(turn.cost_usd, 0.0);
        assert_eq!(turn.tool_calls.len(), 2);
        let (a, b) = (&turn.tool_calls[0], &turn.tool_calls[1]);
        assert_eq!(
            (
                a.kind.as_str(),
                a.command.as_deref(),
                a.exit_code,
                a.status.as_str()
            ),
            ("execute", Some("npm test"), Some(0), "completed")
        );
        assert_eq!(
            (b.command.as_deref(), b.exit_code),
            (Some("npm run test:fail"), Some(1))
        );
        assert!(b.output_excerpt.contains("1 failed, 1 passed"));
        // What we sent: initialize, session/new with cwd + MCP, the model, the prompt.
        let sent: Vec<Value> = String::from_utf8(sent)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let methods: Vec<&str> = sent
            .iter()
            .filter_map(|m| m.get("method").and_then(Value::as_str))
            .collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "session/new",
                "session/set_config_option",
                "session/prompt"
            ]
        );
        assert_eq!(sent[1]["params"]["cwd"], "/home/u/proj");
        assert_eq!(sent[1]["params"]["mcpServers"][0]["name"], "agent_console");
        assert_eq!(sent[2]["params"]["value"], "opencode/big-pickle");
        assert_eq!(
            sent[3]["params"]["prompt"][0]["text"],
            "Run these two shell commands…"
        );
        // The room feed saw the commands finish with their exit codes.
        let acts = activity.borrow();
        assert!(acts.iter().any(|(k, _, t)| k == "tool" && t == "npm test"));
        assert!(acts
            .iter()
            .any(|(k, _, t)| k == "tool_result" && t == "npm test · exit 0"));
        assert!(acts
            .iter()
            .any(|(k, _, t)| k == "tool_result" && t == "npm run test:fail · exit 1"));
        assert!(acts.iter().any(|(k, _, _)| k == "thinking"));
        assert!(acts.iter().filter(|(k, _, _)| k == "text").count() >= 3);
    }

    #[test]
    fn resume_loads_the_session_and_plan_mode_is_requested_for_read_only_turns() {
        let agent = std::io::Cursor::new(
            concat!(
                "{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"protocolVersion\":1}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":4,\"error\":{\"code\":-32601,\"message\":\"no modes\"}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"stopReason\":\"end_turn\"}}\n"
            )
            .as_bytes()
            .to_vec(),
        );
        let mut sent: Vec<u8> = Vec::new();
        let req = AcpPrompt {
            cwd: "/p",
            prompt: "hi",
            resume: Some("ses_prev"),
            model: None,
            policy: ToolPolicy::ReadOnly,
            mcp_servers: vec![],
        };
        let turn = run_turn(agent, &mut sent, &req, &|_, _, _| {}, &|_| {}).unwrap();
        assert_eq!(
            turn.session_id, "ses_prev",
            "load keeps the id we asked for"
        );
        let sent = String::from_utf8(sent).unwrap();
        assert!(sent.contains("\"method\":\"session/load\""));
        assert!(sent.contains("\"sessionId\":\"ses_prev\""));
        assert!(
            sent.contains("\"session/set_mode\""),
            "read-only turns ask for plan mode"
        );
        assert!(!sent.contains("set_config_option"), "no model, no option");
    }

    #[test]
    fn permission_requests_are_answered_by_the_policy_never_by_asking() {
        let opts = vec![
            json!({"optionId": "a1", "name": "Allow once", "kind": "allow_once"}),
            json!({"optionId": "aa", "name": "Always", "kind": "allow_always"}),
            json!({"optionId": "r1", "name": "Reject", "kind": "reject_once"}),
        ];
        assert_eq!(
            permission_answer(ToolPolicy::Full, "execute", &opts).as_deref(),
            Some("a1")
        );
        assert_eq!(
            permission_answer(ToolPolicy::AcceptEdits, "edit", &opts).as_deref(),
            Some("a1")
        );
        assert_eq!(
            permission_answer(ToolPolicy::AcceptEdits, "execute", &opts).as_deref(),
            Some("r1")
        );
        assert_eq!(
            permission_answer(ToolPolicy::ReadOnly, "read", &opts).as_deref(),
            Some("a1")
        );
        assert_eq!(
            permission_answer(ToolPolicy::ReadOnly, "edit", &opts).as_deref(),
            Some("r1")
        );
        assert_eq!(
            permission_answer(ToolPolicy::Plan, "execute", &opts).as_deref(),
            Some("r1")
        );
        assert_eq!(permission_answer(ToolPolicy::Full, "execute", &[]), None);
        // Over the wire: the agent asks mid-turn, the answer carries its id.
        let agent = std::io::Cursor::new(
            concat!(
                "{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"sessionId\":\"s\"}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":77,\"method\":\"session/request_permission\",\"params\":{\"sessionId\":\"s\",\"toolCall\":{\"toolCallId\":\"c\",\"title\":\"rm -rf build\",\"kind\":\"execute\"},\"options\":[{\"optionId\":\"ok\",\"kind\":\"allow_once\"},{\"optionId\":\"no\",\"kind\":\"reject_once\"}]}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"stopReason\":\"end_turn\"}}\n"
            )
            .as_bytes()
            .to_vec(),
        );
        let mut sent: Vec<u8> = Vec::new();
        let req = AcpPrompt {
            cwd: "/p",
            prompt: "x",
            resume: None,
            model: None,
            policy: ToolPolicy::AcceptEdits,
            mcp_servers: vec![],
        };
        run_turn(agent, &mut sent, &req, &|_, _, _| {}, &|_| {}).unwrap();
        let answer = String::from_utf8(sent)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|m| m.get("id").and_then(Value::as_u64) == Some(77))
            .expect("the permission request was answered");
        assert_eq!(answer["result"]["outcome"]["outcome"], "selected");
        assert_eq!(
            answer["result"]["outcome"]["optionId"], "no",
            "execute is not allowed under AcceptEdits"
        );
    }
}
