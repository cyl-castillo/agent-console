//! The console as an MCP server (D9 of IA-NATIVA-2026-10.md): an agent the
//! user points at `hook-bridge console-mcp` can read the console's state,
//! list and run the action contract, open sessions, send prompts, read a
//! session's witnessed events, search the memory corpus and verify the
//! ledger — with its own identity (`--caller`), which every write lands in
//! the ledger under (`actor: agent:<caller>`), never as the human.
//!
//! The bridge relays each `tools/call` to the loopback `/console` route
//! (same token and port as `/inject` and `/mcp`). Tools that only the
//! webview can answer (stores are the truth for sessions, actions, shell
//! position) travel as a `console://call` event and come back through the
//! `console_reply` command; the rest are answered here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

/// Tools the bridge advertises for this server. Lockstep with
/// `console_tools()` in hook-bridge: a name listed there but refused here
/// would be a tool that always errors.
pub const TOOL_NAMES: &[&str] = &[
    "console_state",
    "list_actions",
    "invoke_action",
    "list_sessions",
    "new_session",
    "send_prompt",
    "session_events",
    "memory_search",
    "ledger_verify",
    "decline",
    "request_review",
    "verification_status",
];

/// Tools the webview answers (stores are the truth); the rest are local.
const WEBVIEW_TOOLS: &[&str] = &[
    "console_state",
    "list_actions",
    "invoke_action",
    "list_sessions",
    "new_session",
    "send_prompt",
];

/// Action ids an agent may run through `invoke_action`: navigation, shell
/// toggles and harmless app actions. Anything that kills, deletes, pushes
/// or spends stays a human gesture (or its own, scoped tool).
pub fn action_allowed(id: &str) -> bool {
    id.starts_with("nav.")
        || id.starts_with("ui.")
        || matches!(
            id,
            "help.getting_started"
                | "git.refresh"
                | "app.check_updates"
                | "prompt.composer"
                | "modules.configure"
                | "session.new"
        )
}

/// How long a webview roundtrip may take before the agent gets an error.
const WEBVIEW_TIMEOUT: Duration = Duration::from_secs(8);

/// One relayed `tools/call`, as the bridge posts it to `/console`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleCall {
    /// Project root the agent was configured for.
    pub project: String,
    /// The agent's identity (`--caller`): what the ledger records as actor.
    pub caller: String,
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

/// What the webview receives for a tool it answers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebviewCall {
    pub id: u64,
    pub caller: String,
    pub tool: String,
    pub arguments: Value,
}

#[derive(Default)]
pub struct ConsoleMcp {
    pending: Mutex<HashMap<u64, mpsc::SyncSender<Value>>>,
    next: AtomicU64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

impl ConsoleMcp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dispatch one call. Every accepted call — answered or refused by the
    /// webview — is recorded in the ledger under the agent's identity.
    pub fn call(&self, app: &AppHandle, req: &ConsoleCall) -> Result<Value, String> {
        if !TOOL_NAMES.contains(&req.name.as_str()) {
            return Err("Unknown tool".into());
        }
        if req.caller.trim().is_empty() {
            return Err("Missing caller identity".into());
        }
        if !req.arguments.is_object() && !req.arguments.is_null() {
            return Err("Invalid arguments".into());
        }
        if req.name == "invoke_action" {
            let id = arg_str(&req.arguments, "id").ok_or("Missing action id")?;
            if !action_allowed(id) {
                return Err(format!(
                    "Action {id} is not available to agents; it stays a human gesture"
                ));
            }
        }
        // D14: the caller's identity, witnessed once per project under its own
        // actor — an MCP agent never appears as the human.
        {
            let state = app.state::<AppState>();
            let card = crate::services::identity_service::card_for_mcp_caller(
                &req.caller,
                now_ms(),
                &crate::commands::identity::issuer(),
            );
            let _ = state.testigo.on_agent_identity(
                &req.project,
                now_ms(),
                &card.agent_id,
                None,
                crate::services::identity_service::ledger_payload(&card, None),
            );
        }
        if req.name == "decline" {
            // Its own first-class ledger event (D13), not a generic action.
            return self.decline(app, req);
        }
        if req.name == "request_review" {
            // D10: the verdict is its own ledger event under the reviewer's
            // identity; the request itself is witnessed as the caller's act.
            let outcome = self.request_review(app, req);
            self.witness(app, req, &outcome);
            return outcome;
        }
        let outcome = if WEBVIEW_TOOLS.contains(&req.name.as_str()) {
            self.ask_webview(app, req)
        } else {
            self.answer_locally(app, req)
        };
        self.witness(app, req, &outcome);
        outcome
    }

    fn ask_webview(&self, app: &AppHandle, req: &ConsoleCall) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::sync_channel::<Value>(1);
        self.pending.lock().insert(id, tx);
        let call = WebviewCall {
            id,
            caller: req.caller.clone(),
            tool: req.name.clone(),
            arguments: if req.arguments.is_null() {
                json!({})
            } else {
                req.arguments.clone()
            },
        };
        if app.emit("console://call", &call).is_err() {
            self.pending.lock().remove(&id);
            return Err("The console window is not available".into());
        }
        let answer = rx.recv_timeout(WEBVIEW_TIMEOUT);
        self.pending.lock().remove(&id);
        let answer = answer.map_err(|_| "The console did not answer in time".to_string())?;
        if answer.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(answer.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(answer
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("The console refused the call")
                .to_string())
        }
    }

    /// The webview's answer to a pending call (`console_reply` command).
    /// Unknown ids (timed out, duplicate) are dropped silently.
    pub fn reply(&self, id: u64, result: Value) {
        if let Some(tx) = self.pending.lock().remove(&id) {
            let _ = tx.send(result);
        }
    }

    fn answer_locally(&self, app: &AppHandle, req: &ConsoleCall) -> Result<Value, String> {
        let state = app.state::<AppState>();
        match req.name.as_str() {
            "session_events" => {
                let session = arg_str(&req.arguments, "session").ok_or("Missing session")?;
                let limit = req
                    .arguments
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(200)
                    .clamp(1, 1000) as usize;
                let events = state
                    .testigo
                    .list(&req.project, None, None)
                    .map_err(|e| e.to_string())?;
                let mine: Vec<&_> = events
                    .iter()
                    .filter(|e| e.term_id.as_deref() == Some(session))
                    .collect();
                let start = mine.len().saturating_sub(limit);
                let mine: Vec<Value> = mine[start..]
                    .iter()
                    .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
                    .collect();
                Ok(json!({ "session": session, "events": mine }))
            }
            "memory_search" => {
                let query = arg_str(&req.arguments, "query").ok_or("Missing query")?;
                let k = req
                    .arguments
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize);
                let hits = crate::commands::semantic::semantic_search(
                    req.project.clone(),
                    query.to_string(),
                    k,
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({ "hits": hits }))
            }
            "ledger_verify" => {
                let report = state
                    .testigo
                    .verify(&req.project)
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::to_value(report).unwrap_or(Value::Null))
            }
            // F1c/F1d: the console's own verdict on a turn's claims, so an
            // agent can check its last turn before declaring it done — and
            // run the gate it did not run instead of asserting green.
            "verification_status" => {
                let session = arg_str(&req.arguments, "session").ok_or("Missing session")?;
                let turn = arg_str(&req.arguments, "turn");
                let events = state
                    .testigo
                    .list(&req.project, None, None)
                    .map_err(|e| e.to_string())?;
                let found = events.iter().rev().find(|e| {
                    e.kind == "verification"
                        && e.term_id.as_deref() == Some(session)
                        && turn.is_none_or(|t| e.turn_id.as_deref() == Some(t))
                });
                match found {
                    Some(v) => {
                        let decided = events.iter().any(|e| {
                            e.kind == "verification_decision"
                                && e.turn_id == v.turn_id
                                && e.seq > v.seq
                        });
                        let mut out = v.payload.clone();
                        if let Some(o) = out.as_object_mut() {
                            o.insert("turn".into(), json!(v.turn_id));
                            o.insert("seq".into(), json!(v.seq));
                            o.insert("decidedByHuman".into(), json!(decided));
                            o.insert("note".into(), json!(
                                "Only `verified` means a check passed with exit 0 after the last edit. If a gate is not_run, run it and report the exit code instead of asserting green."
                            ));
                        }
                        Ok(out)
                    }
                    None => Ok(json!({
                        "status": "none",
                        "note": "No finished turn with a verification line for this session yet."
                    })),
                }
            }
            other => Err(format!("Unknown tool {other}")),
        }
    }

    /// D13: the agent declines or escalates. Witnessed as `agent_declined`
    /// under its identity, surfaced to the human as a decision in the Inbox,
    /// and never retried by the console: the reason travels, the task waits.
    fn decline(&self, app: &AppHandle, req: &ConsoleCall) -> Result<Value, String> {
        let kind = arg_str(&req.arguments, "kind").unwrap_or("decline");
        if kind != "decline" && kind != "escalate" {
            return Err("kind must be decline or escalate".into());
        }
        let reason = arg_str(&req.arguments, "reason").ok_or("A reason is required")?;
        let suggestion = arg_str(&req.arguments, "suggestion");
        let session = arg_str(&req.arguments, "session");
        let ts = now_ms();
        let state = app.state::<AppState>();
        let ev = state
            .testigo
            .on_agent_declined(
                &req.project,
                ts,
                &req.caller,
                kind,
                reason,
                suggestion,
                session,
            )
            .map_err(|e| e.to_string())?;
        let payload = json!({
            "seq": ev.seq,
            "ts": ts,
            "caller": req.caller,
            "kind": kind,
            "reason": reason,
            "suggestion": suggestion,
            "session": session,
            "project": req.project,
        });
        let _ = app.emit("console://declined", &payload);
        Ok(json!({
            "recorded": true,
            "seq": ev.seq,
            "note": "The human will see this in the Inbox. The console will not retry the task on its own.",
        }))
    }

    /// D10 over MCP: an agent asks for an adversarial review of a turn (its
    /// own, typically) by a different identity. The run is blocking for the
    /// caller — the reviewer reads the diff and answers — and the verdict
    /// lands in the ledger and the Inbox either way.
    fn request_review(&self, app: &AppHandle, req: &ConsoleCall) -> Result<Value, String> {
        let session = arg_str(&req.arguments, "session").ok_or("Missing session")?;
        let turn = arg_str(&req.arguments, "turn");
        let state = app.state::<AppState>();
        let verdict = crate::services::reviewer_service::review(
            &state.testigo,
            &req.project,
            session,
            turn,
            now_ms(),
        )
        .map_err(|e| e.to_string())?;
        let _ = app.emit(
            "console://review",
            &json!({ "project": req.project, "verdict": verdict }),
        );
        Ok(json!({
            "verdict": verdict.verdict,
            "findings": verdict.findings,
            "reviewer": verdict.reviewer,
            "seq": verdict.seq,
            "note": "The human sees this verdict in the Inbox. A block stands until a human overrides it with a reason.",
        }))
    }

    /// Record the call in the ledger as the agent's act. A failure to witness
    /// never changes the answer (the ledger's witness switch may be off).
    fn witness(&self, app: &AppHandle, req: &ConsoleCall, outcome: &Result<Value, String>) {
        let state = app.state::<AppState>();
        // The session the act concerns, when the tool names one: the ledger
        // map then shows the agent's hand on that session's lane.
        let term_id = match req.name.as_str() {
            "send_prompt" | "session_events" | "request_review" => {
                arg_str(&req.arguments, "session").map(String::from)
            }
            "new_session" => outcome
                .as_ref()
                .ok()
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                .map(String::from),
            _ => None,
        };
        let summary = match req.name.as_str() {
            "invoke_action" => arg_str(&req.arguments, "id").unwrap_or("").to_string(),
            "send_prompt" => arg_str(&req.arguments, "text")
                .unwrap_or("")
                .chars()
                .take(200)
                .collect(),
            "memory_search" => arg_str(&req.arguments, "query").unwrap_or("").to_string(),
            "new_session" => arg_str(&req.arguments, "name").unwrap_or("").to_string(),
            _ => String::new(),
        };
        let _ = state.testigo.on_agent_action(
            &req.project,
            now_ms(),
            &req.caller,
            &req.name,
            &summary,
            outcome.is_ok(),
            outcome.as_ref().err().map(String::as_str),
            term_id.as_deref(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_only_run_harmless_actions() {
        assert!(action_allowed("nav.proof"));
        assert!(action_allowed("ui.toggle_v2"));
        assert!(action_allowed("session.new"));
        assert!(!action_allowed("session.close"));
        assert!(!action_allowed("git.push"));
        assert!(!action_allowed("snapshot.restore_latest"));
    }

    #[test]
    fn a_reply_resolves_its_pending_call_and_unknown_ids_are_dropped() {
        let mcp = ConsoleMcp::new();
        let (tx, rx) = mpsc::sync_channel(1);
        mcp.pending.lock().insert(7, tx);
        mcp.reply(99, json!({ "ok": true })); // nobody waits for 99
        mcp.reply(7, json!({ "ok": true, "result": 1 }));
        assert_eq!(rx.recv().unwrap()["result"], 1);
        assert!(mcp.pending.lock().is_empty() || !mcp.pending.lock().contains_key(&7));
    }
}
