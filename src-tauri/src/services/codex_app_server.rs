//! A minimal client for `codex app-server` (COMPETENCIA-2026-10 F5): JSONL
//! JSON-RPC over the server's stdio, enough to run one turn of a room or a
//! job and witness it per command — `initialize` + `initialized` →
//! `thread/start` (or `thread/resume`) → `turn/start`, with `item/*`
//! notifications folded into the room's activity, approvals answered by the
//! turn's `ToolPolicy`, token usage and the account's rate limits read from
//! their notifications. `codex mcp-server` is gone (0.15x) and `codex exec
//! --json` shows a command but never its exit; the app-server shows both.
//! Verified against Codex 0.163.0: `fixtures/appserver/codex-0.163.0-turn.jsonl`
//! is a real turn.
//!
//! Generic over the streams so the fixture drives the tests without a
//! process; `engine_runner` wires it to a child's stdin/stdout.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::engine_runner::{ActivitySink, ToolPolicy};

const ID_INIT: u64 = 0;
const ID_THREAD: u64 = 1;
const ID_TURN: u64 = 2;
const EXCERPT_MAX: usize = 400;

pub struct AppServerPrompt<'a> {
    pub cwd: &'a str,
    pub prompt: &'a str,
    /// A thread id from a prior turn: `thread/resume` instead of `thread/start`.
    pub resume: Option<&'a str>,
    /// Reasoning effort (`low` / `medium` / `high`), empty = server default.
    pub effort: &'a str,
    pub policy: ToolPolicy,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppServerCommand {
    pub id: String,
    pub command: String,
    /// inProgress · completed · failed · declined
    pub status: String,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
    pub output_excerpt: String,
}

/// The account's quota as the server reports it (`account/rateLimits/updated`):
/// the Codex twin of Claude's status-line windows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppServerQuota {
    pub primary_pct: Option<f64>,
    pub primary_window_mins: Option<u64>,
    pub primary_resets_at: Option<i64>,
    pub secondary_pct: Option<f64>,
    pub secondary_window_mins: Option<u64>,
    pub secondary_resets_at: Option<i64>,
    pub plan_type: Option<String>,
}

#[derive(Debug, Default)]
pub struct AppServerTurn {
    pub text: String,
    pub thread_id: String,
    pub turn_id: String,
    /// completed · interrupted · failed
    pub status: String,
    /// input + output (+ reasoning) tokens of the turn, cached reads excluded.
    pub tokens: u64,
    pub commands: Vec<AppServerCommand>,
    pub quota: Option<AppServerQuota>,
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({ "method": method, "id": id, "params": params })
}

fn send<W: Write>(w: &mut W, msg: &Value) -> AppResult<()> {
    w.write_all(msg.to_string().as_bytes())?;
    w.write_all(b"\n")?;
    w.flush()?;
    Ok(())
}

/// Sandbox and approval policy for the thread, from the turn's policy. The
/// room's policy is the human's standing decision: nothing asks mid-turn.
fn thread_params(req: &AppServerPrompt<'_>) -> (&'static str, &'static str) {
    match req.policy {
        ToolPolicy::Full => ("danger-full-access", "never"),
        ToolPolicy::AcceptEdits => ("workspace-write", "never"),
        ToolPolicy::ReadOnly | ToolPolicy::Plan => ("read-only", "never"),
    }
}

/// What an approval request gets under this policy.
pub fn approval_decision(policy: ToolPolicy, method: &str) -> &'static str {
    let is_command = method.contains("commandExecution");
    match policy {
        ToolPolicy::Full => "accept",
        ToolPolicy::AcceptEdits if !is_command => "accept",
        _ => "decline",
    }
}

fn excerpt(s: &str) -> String {
    if s.chars().count() > EXCERPT_MAX {
        s.chars().take(EXCERPT_MAX).collect()
    } else {
        s.to_string()
    }
}

/// `/bin/bash -lc 'npm test'` → `npm test`; anything else as is.
pub fn plain_command(cmd: &str) -> String {
    let t = cmd.trim();
    for prefix in [
        "/bin/bash -lc ",
        "/bin/sh -lc ",
        "bash -lc ",
        "sh -lc ",
        "/bin/bash -c ",
        "bash -c ",
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let r = rest.trim();
            if (r.starts_with('\'') && r.ends_with('\'') || r.starts_with('"') && r.ends_with('"'))
                && r.len() >= 2
            {
                return r[1..r.len() - 1].to_string();
            }
            return r.to_string();
        }
    }
    t.to_string()
}

/// Run one turn over the server's streams. `on_thread` fires as soon as the
/// thread id is known (the room keeps it to resume next turn).
pub fn run_turn<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    req: &AppServerPrompt<'_>,
    on_activity: &ActivitySink,
) -> AppResult<AppServerTurn> {
    send(
        &mut writer,
        &rpc(
            ID_INIT,
            "initialize",
            json!({ "clientInfo": { "name": "agent-console", "title": "Agent Console", "version": env!("CARGO_PKG_VERSION") } }),
        ),
    )?;
    let mut turn = AppServerTurn::default();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(AppError::Other(
                "codex app-server closed its stream before finishing the turn".into(),
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
            (Some(ID_INIT), None) => {
                if let Some(e) = msg.get("error") {
                    return Err(AppError::Other(format!(
                        "codex app-server initialize failed: {e}"
                    )));
                }
                send(
                    &mut writer,
                    &json!({ "method": "initialized", "params": {} }),
                )?;
                let (sandbox, approval) = thread_params(req);
                match req.resume {
                    Some(tid) => send(
                        &mut writer,
                        &rpc(
                            ID_THREAD,
                            "thread/resume",
                            json!({ "threadId": tid, "excludeTurns": true, "cwd": req.cwd, "approvalPolicy": approval, "sandbox": sandbox }),
                        ),
                    )?,
                    None => send(
                        &mut writer,
                        &rpc(
                            ID_THREAD,
                            "thread/start",
                            json!({ "cwd": req.cwd, "approvalPolicy": approval, "sandbox": sandbox, "serviceName": "agent-console" }),
                        ),
                    )?,
                }
            }
            (Some(ID_THREAD), None) => {
                if let Some(e) = msg.get("error") {
                    return Err(AppError::Other(format!(
                        "codex app-server thread failed: {e}"
                    )));
                }
                let tid = msg
                    .pointer("/result/thread/id")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .or_else(|| req.resume.map(String::from))
                    .ok_or_else(|| AppError::Other("codex app-server thread has no id".into()))?;
                turn.thread_id = tid.clone();
                let mut params =
                    json!({ "threadId": tid, "input": [{ "type": "text", "text": req.prompt }] });
                if !req.effort.is_empty() {
                    params["effort"] = json!(req.effort);
                }
                send(&mut writer, &rpc(ID_TURN, "turn/start", params))?;
            }
            (Some(ID_TURN), None) => {
                if let Some(e) = msg.get("error") {
                    return Err(AppError::Other(format!(
                        "codex app-server turn failed: {e}"
                    )));
                }
                if let Some(t) = msg.pointer("/result/turn/id").and_then(Value::as_str) {
                    turn.turn_id = t.to_string();
                }
            }
            // --- server → client requests: approvals ---
            (Some(rid), Some(m)) if m.ends_with("/requestApproval") => {
                let decision = approval_decision(req.policy, m);
                let what = msg
                    .pointer("/params/command")
                    .map(|c| c.to_string())
                    .or_else(|| msg.pointer("/params/itemId").map(|c| c.to_string()))
                    .unwrap_or_default();
                on_activity("permission", m, &format!("{what} → {decision}"));
                send(
                    &mut writer,
                    &json!({ "id": rid, "result": { "decision": decision } }),
                )?;
            }
            (Some(rid), Some(other)) => {
                send(
                    &mut writer,
                    &json!({ "id": rid, "error": { "code": -32601, "message": format!("{other} not supported") } }),
                )?;
            }
            // --- notifications ---
            (None, Some("item/started")) | (None, Some("item/completed")) => {
                let Some(item) = msg.pointer("/params/item") else {
                    continue;
                };
                fold_item(
                    item,
                    method == Some("item/completed"),
                    &mut turn,
                    on_activity,
                );
            }
            (None, Some("thread/tokenUsage/updated")) => {
                if let Some(t) = msg.pointer("/params/tokenUsage/total") {
                    let n = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
                    // Real new tokens: input minus cached reads, plus output.
                    turn.tokens =
                        n("inputTokens").saturating_sub(n("cachedInputTokens")) + n("outputTokens");
                }
            }
            (None, Some("account/rateLimits/updated")) => {
                if let Some(rl) = msg.pointer("/params/rateLimits") {
                    let win = |k: &str| rl.get(k).filter(|v| !v.is_null());
                    let pct = |w: Option<&Value>| {
                        w.and_then(|v| v.get("usedPercent")).and_then(Value::as_f64)
                    };
                    let mins = |w: Option<&Value>| {
                        w.and_then(|v| v.get("windowDurationMins"))
                            .and_then(Value::as_u64)
                    };
                    let resets = |w: Option<&Value>| {
                        w.and_then(|v| v.get("resetsAt")).and_then(Value::as_i64)
                    };
                    turn.quota = Some(AppServerQuota {
                        primary_pct: pct(win("primary")),
                        primary_window_mins: mins(win("primary")),
                        primary_resets_at: resets(win("primary")),
                        secondary_pct: pct(win("secondary")),
                        secondary_window_mins: mins(win("secondary")),
                        secondary_resets_at: resets(win("secondary")),
                        plan_type: rl.get("planType").and_then(Value::as_str).map(String::from),
                    });
                }
            }
            (None, Some("turn/completed")) => {
                turn.status = msg
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(e) = msg.pointer("/params/turn/error").filter(|e| !e.is_null()) {
                    return Err(AppError::Other(format!("codex app-server turn error: {e}")));
                }
                return Ok(turn);
            }
            _ => {}
        }
    }
}

fn fold_item(item: &Value, completed: bool, turn: &mut AppServerTurn, on_activity: &ActivitySink) {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "agentMessage" if completed => {
            {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    // The final answer wins; commentary along the way is shown, not kept.
                    let phase = item
                        .get("phase")
                        .and_then(Value::as_str)
                        .unwrap_or("final_answer");
                    if !t.is_empty() {
                        on_activity("text", "", t);
                    }
                    if phase == "final_answer" || turn.text.is_empty() {
                        turn.text = t.to_string();
                    }
                }
            }
        }
        "reasoning" if completed => {
            let summary = item
                .get("summary")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|s| {
                            s.get("text").and_then(Value::as_str).or_else(|| s.as_str())
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if !summary.trim().is_empty() {
                on_activity("thinking", "", &excerpt(&summary));
            }
        }
        "commandExecution" => {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let command = plain_command(item.get("command").and_then(Value::as_str).unwrap_or(""));
            if !completed {
                on_activity("tool", "shell", &command);
                turn.commands.push(AppServerCommand {
                    id,
                    command,
                    status: "inProgress".into(),
                    ..Default::default()
                });
                return;
            }
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("completed")
                .to_string();
            let exit_code = item.get("exitCode").and_then(Value::as_i64);
            let entry = AppServerCommand {
                id: id.clone(),
                command: command.clone(),
                status: status.clone(),
                exit_code,
                duration_ms: item.get("durationMs").and_then(Value::as_u64),
                output_excerpt: excerpt(
                    item.get("aggregatedOutput")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                ),
            };
            match turn.commands.iter_mut().find(|c| c.id == id) {
                Some(c) => *c = entry,
                None => turn.commands.push(entry),
            }
            let verdict = match (exit_code, status.as_str()) {
                (Some(c), _) => format!(" · exit {c}"),
                (None, "declined") => " · declined".to_string(),
                (None, "failed") => " · failed".to_string(),
                _ => String::new(),
            };
            on_activity("tool_result", "shell", &format!("{command}{verdict}"));
        }
        "fileChange" => {
            let path = item.get("path").and_then(Value::as_str).unwrap_or("");
            on_activity(if completed { "tool_result" } else { "tool" }, "edit", path);
        }
        "mcpToolCall" => {
            let name = item.get("tool").and_then(Value::as_str).unwrap_or("mcp");
            on_activity(if completed { "tool_result" } else { "tool" }, "mcp", name);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const TURN: &str = include_str!("../../fixtures/appserver/codex-0.163.0-turn.jsonl");

    fn server_lines() -> Vec<u8> {
        TURN.lines()
            .filter_map(|l| l.strip_prefix("< "))
            .map(|l| format!("{l}\n"))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn a_real_codex_turn_is_replayed_with_its_commands_exit_codes_tokens_and_quota() {
        let server = std::io::Cursor::new(server_lines());
        let mut sent: Vec<u8> = Vec::new();
        let activity = RefCell::new(Vec::<(String, String, String)>::new());
        let req = AppServerPrompt {
            cwd: "/home/u/proj",
            prompt: "Run these two shell commands…",
            resume: None,
            effort: "medium",
            policy: ToolPolicy::AcceptEdits,
        };
        let turn = run_turn(server, &mut sent, &req, &|k, l, t| {
            activity.borrow_mut().push((k.into(), l.into(), t.into()))
        })
        .unwrap();
        assert_eq!(turn.thread_id, "01a16b3f-aa4f-75a1-b0ab-0f4d5a09f354");
        assert_eq!(turn.turn_id, "01a16b3f-adbf-7142-8768-da3ddcdc3752");
        assert_eq!(turn.status, "completed");
        assert_eq!(
            turn.text, "All tests pass.",
            "the final answer, not the commentary"
        );
        // input − cached + output of the last tokenUsage total.
        assert_eq!(turn.tokens, 58605 - 45056 + 360);
        let cmds: Vec<(&str, Option<i64>, &str)> = turn
            .commands
            .iter()
            .map(|c| (c.command.as_str(), c.exit_code, c.status.as_str()))
            .collect();
        assert!(cmds.contains(&("npm test", Some(0), "completed")));
        assert!(cmds.contains(&("npm run test:fail", Some(1), "failed")));
        assert!(turn
            .commands
            .iter()
            .any(|c| c.output_excerpt.contains("1 failed, 1 passed")));
        let q = turn.quota.unwrap();
        assert_eq!(q.primary_pct, Some(5.0));
        assert_eq!(q.primary_window_mins, Some(300));
        assert_eq!(q.secondary_window_mins, Some(10080));
        assert_eq!(q.plan_type.as_deref(), Some("team"));
        // What we sent, in order.
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
            ["initialize", "initialized", "thread/start", "turn/start"]
        );
        assert_eq!(sent[2]["params"]["sandbox"], "workspace-write");
        assert_eq!(sent[2]["params"]["approvalPolicy"], "never");
        assert_eq!(sent[3]["params"]["effort"], "medium");
        assert_eq!(
            sent[3]["params"]["input"][0]["text"],
            "Run these two shell commands…"
        );
        let acts = activity.borrow();
        assert!(acts
            .iter()
            .any(|(k, _, t)| k == "tool_result" && t == "npm test · exit 0"));
        assert!(acts
            .iter()
            .any(|(k, _, t)| k == "tool_result" && t == "npm run test:fail · exit 1"));
        assert!(acts
            .iter()
            .any(|(k, _, t)| k == "text" && t == "All tests pass."));
    }

    #[test]
    fn resume_uses_thread_resume_and_approvals_follow_the_policy() {
        let server = std::io::Cursor::new(
            concat!(
                "{\"id\":0,\"result\":{\"userAgent\":\"x\"}}\n",
                "{\"id\":1,\"result\":{\"thread\":{\"id\":\"thr_prev\"}}}\n",
                "{\"id\":2,\"result\":{\"turn\":{\"id\":\"turn_1\",\"status\":\"inProgress\"}}}\n",
                "{\"method\":\"item/commandExecution/requestApproval\",\"id\":40,\"params\":{\"itemId\":\"i1\",\"command\":[\"rm\",\"-rf\",\"build\"]}}\n",
                "{\"method\":\"item/fileChange/requestApproval\",\"id\":41,\"params\":{\"itemId\":\"i2\"}}\n",
                "{\"method\":\"turn/completed\",\"params\":{\"turn\":{\"id\":\"turn_1\",\"status\":\"completed\",\"error\":null}}}\n"
            )
            .as_bytes()
            .to_vec(),
        );
        let mut sent: Vec<u8> = Vec::new();
        let req = AppServerPrompt {
            cwd: "/p",
            prompt: "x",
            resume: Some("thr_prev"),
            effort: "",
            policy: ToolPolicy::AcceptEdits,
        };
        let turn = run_turn(server, &mut sent, &req, &|_, _, _| {}).unwrap();
        assert_eq!(turn.thread_id, "thr_prev");
        let sent: Vec<Value> = String::from_utf8(sent)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(sent
            .iter()
            .any(|m| m.get("method").and_then(Value::as_str) == Some("thread/resume")));
        let answer = |id: u64| {
            sent.iter()
                .find(|m| m.get("id").and_then(Value::as_u64) == Some(id))
                .unwrap()["result"]["decision"]
                .clone()
        };
        assert_eq!(answer(40), "decline", "no shell under AcceptEdits");
        assert_eq!(answer(41), "accept", "edits are fine under AcceptEdits");
        assert_eq!(
            approval_decision(ToolPolicy::Full, "item/commandExecution/requestApproval"),
            "accept"
        );
        assert_eq!(
            approval_decision(ToolPolicy::ReadOnly, "item/fileChange/requestApproval"),
            "decline"
        );
        assert_eq!(plain_command("/bin/bash -lc 'npm test'"), "npm test");
        assert_eq!(
            plain_command("/bin/bash -lc \"sed -n '1,3p' x\""),
            "sed -n '1,3p' x"
        );
        assert_eq!(plain_command("npm test"), "npm test");
    }
}
