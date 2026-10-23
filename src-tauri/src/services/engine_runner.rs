//! Engine-neutral turn runner.
//!
//! The orchestrator (roundtable / future room) does not care whether a
//! participant is backed by Claude or Codex — it builds a prompt, asks the
//! engine to run one headless turn, and gets back a normalized [`TurnOutput`]
//! plus a stream of activity callbacks. Each engine has its own CLI, flags, and
//! JSONL event shape; the adapters here translate both into one model.
//!
//! Normalized event mapping (confirmed empirically against both CLIs):
//!
//! | normalized   | Claude (`-p --output-format stream-json`) | Codex (`exec --json`)                   | OpenCode (`run --format json`)            |
//! |--------------|-------------------------------------------|-----------------------------------------|-------------------------------------------|
//! | resume id    | `result.session_id`                       | `thread.started.thread_id`              | `sessionID` on every event; text mode: `session list` by title |
//! | final text   | `result.result`                           | `item.completed{agent_message}.text`    | concatenated `text` parts (`part.time.end` set) |
//! | tool call    | assistant `tool_use` block                | `item.started{command_execution}`       | `tool_use` part (`part.tool`)              |
//! | tokens       | `result.usage` (sans cache reads)         | `turn.completed.usage` (sans cache)     | `step_finish.part.tokens.{input,output}`  |
//! | live pulse   | `text_delta` (per token)                  | `item.started` (per item — coarser)     | `step_start`/`tool_use`/`text` (per part) |
//! | cost (USD)   | `result.total_cost_usd`                   | not reported → 0.0                       | `step_finish.part.cost`, summed           |
//!
//! OpenCode has no hooks, so a turn of it leaves in the ledger only what this
//! runner reports — no per-tool witness. Its MCP entry lives in
//! `opencode.json` of the working folder (see `opencode_service`), shared by
//! every participant there, so the connector identifies an OpenCode caller by
//! its session id (`opencode_identity`) rather than by `--caller`.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::services::claude_cli;

/// Which CLI backs a participant. Defaults to Claude so payloads that predate
/// the field (every existing roundtable config) still deserialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    #[default]
    Claude,
    Codex,
    Opencode,
}

impl Engine {
    /// Binary base name, as typed on a terminal.
    pub fn bin_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }
}

/// How much the agent is allowed to do during a turn. Maps to each CLI's
/// permission/sandbox flags. `AcceptEdits`/`Full` are wired through both
/// adapters but unused while the room is conversation-only (read-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToolPolicy {
    /// Read & reason only — no edits, no shell. The conversational-room default:
    /// Claude runs with no permission flag (headless auto-denies edits), Codex
    /// with `-s read-only`.
    ReadOnly,
    /// File edits allowed, no arbitrary shell (Claude `acceptEdits` / Codex
    /// `-s workspace-write`).
    AcceptEdits,
    /// Everything, including shell (Claude `--dangerously-skip-permissions` /
    /// Codex `--dangerously-bypass-approvals-and-sandbox`).
    Full,
    /// Plan mode: the agent may read and reason but Claude refuses every
    /// mutation by construction (`--permission-mode plan`) — what every
    /// unattended run (advisor, reflect, curator, scheduler) runs under.
    /// Codex has no plan mode; `-s read-only` is the closest sandbox.
    Plan,
}

/// Where a runner parks the child process for the duration of a turn, so the
/// orchestrator can kill it from another thread (stop, discard, idle
/// watchdog). The runner puts the `Child` in right after spawn and takes it
/// back to `wait()`; a kill in between makes the read loop hit EOF and the
/// wait report the signal. Nobody but the runner ever *takes* the child —
/// callers only `kill()` through the guard — so there is no reap race.
pub type ChildSlot = parking_lot::Mutex<Option<std::process::Child>>;

/// Kill whatever child is parked in `slot`, if any. Idempotent; a child that
/// already exited is not an error.
pub fn kill_parked(slot: &ChildSlot) {
    if let Some(child) = slot.lock().as_mut() {
        let _ = child.kill();
    }
}

/// Everything a single headless turn needs, independent of engine.
pub struct RunCtx<'a> {
    /// Working directory the turn runs in.
    pub cwd: &'a Path,
    /// Claude: model alias (`opus`/`sonnet`). Codex: reasoning effort
    /// (`low`/`medium`/`high`). Validated shell-safe before reaching here.
    pub model: &'a str,
    /// What the agent may do this turn.
    pub tools: ToolPolicy,
    pub prompt: &'a str,
    /// Resume id from a prior turn of the SAME participant, to retain its memory.
    pub resume: Option<&'a str>,
    /// Where to park the child so the caller can kill it mid-turn. `None`
    /// keeps it private to the runner (nothing can interrupt the turn).
    pub child_slot: Option<&'a ChildSlot>,
    /// Give the turn the connector's MCP server (`delegate_task`, `ask_user`,
    /// …) speaking for this participant. `None` = a plain turn, exactly as
    /// before the connector existed.
    pub mcp: Option<McpAttach<'a>>,
}

/// The `agent_console` MCP server a room turn is given: the hook-bridge
/// binary in `mcp` mode, addressed to one participant of one room. Both CLIs
/// get the same command line; only the config syntax differs.
#[derive(Debug, Clone, Copy)]
pub struct McpAttach<'a> {
    /// Stable path of the hook-bridge sidecar (`HooksRuntime::bridge_binary`).
    pub bridge: &'a Path,
    /// Project root the room belongs to.
    pub project: &'a str,
    /// Room id.
    pub job: &'a str,
    /// Participant id this server speaks for.
    pub caller: &'a str,
}

/// Name both CLIs register the server under; tool names become
/// `mcp__agent_console__<tool>` on the Claude side.
pub const MCP_SERVER_NAME: &str = "agent_console";

impl McpAttach<'_> {
    fn bridge_args(&self) -> [String; 7] {
        [
            "mcp".into(),
            "--project".into(),
            self.project.into(),
            "--job".into(),
            self.job.into(),
            "--caller".into(),
            self.caller.into(),
        ]
    }

    /// `--mcp-config` takes a path OR an inline JSON document; inline keeps
    /// the per-turn identity out of the filesystem. The user's own servers
    /// stay loaded (no `--strict-mcp-config`): a room must not silently lose
    /// the MCPs the user configured.
    fn claude_config_json(&self) -> String {
        serde_json::json!({
            "mcpServers": {
                MCP_SERVER_NAME: {
                    "command": self.bridge.to_string_lossy(),
                    "args": self.bridge_args(),
                }
            }
        })
        .to_string()
    }

    /// Codex `-c` override as a TOML inline table. Strings are emitted with
    /// JSON escaping, which TOML basic strings accept verbatim — so a Windows
    /// path with backslashes survives. `default_tools_approval_mode="approve"`
    /// pre-approves our tools: `exec` is non-interactive and would otherwise
    /// auto-deny the first `delegate_task`.
    fn codex_config_override(&self) -> String {
        let quoted = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
        let args = self
            .bridge_args()
            .iter()
            .map(|a| quoted(a))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "mcp_servers.{MCP_SERVER_NAME}={{command={},args=[{args}],required=true,default_tools_approval_mode=\"approve\"}}",
            quoted(&self.bridge.to_string_lossy())
        )
    }
}

/// Normalized result of one turn, regardless of engine.
pub struct TurnOutput {
    pub text: String,
    /// Id to resume this participant next turn (Claude session / Codex thread).
    pub session_id: Option<String>,
    /// Real new tokens (excludes cache reads) for the budget.
    pub tokens: u64,
    /// Dollar cost as reported by the CLI. Codex does not report one → 0.0.
    pub cost_usd: f64,
}

/// Callback for live activity within a turn: `(kind, label, text)` where kind is
/// "thinking" | "tool" | "text". The orchestrator forwards these to the UI.
pub type ActivitySink<'a> = dyn Fn(&str, &str, &str) + 'a;

pub trait EngineRunner {
    fn run(&self, ctx: &RunCtx, on_activity: &ActivitySink) -> AppResult<TurnOutput>;
}

/// Dispatch to the adapter for `engine`. The runners are stateless, so a
/// `'static` reference is enough and avoids boxing.
pub fn runner_for(engine: Engine) -> &'static dyn EngineRunner {
    match engine {
        Engine::Claude => &ClaudeRunner,
        Engine::Codex => &CodexRunner,
        Engine::Opencode => &OpencodeRunner,
    }
}

// ---------------- OpenCode ----------------

pub struct OpencodeRunner;

/// Who an OpenCode session speaks for. Filled when the runner creates or
/// resumes a session with a connector attached; the loopback `/mcp` route
/// resolves a call that carries `_meta["ai.opencode/sessionID"]` through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpencodeIdentity {
    pub project: String,
    pub job: String,
    pub caller: String,
}

type IdentityMap = std::collections::HashMap<String, OpencodeIdentity>;

fn opencode_sessions() -> &'static parking_lot::Mutex<IdentityMap> {
    static MAP: std::sync::OnceLock<parking_lot::Mutex<IdentityMap>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| parking_lot::Mutex::new(std::collections::HashMap::new()))
}

/// The participant whose OpenCode turn is running in each room right now,
/// keyed by room id. A fresh session's id is only known once its first
/// event arrives, and the agent may call the connector before that; the
/// room's turn in flight is the answer then (rooms run one turn at a time).
fn opencode_turns() -> &'static parking_lot::Mutex<IdentityMap> {
    static MAP: std::sync::OnceLock<parking_lot::Mutex<IdentityMap>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| parking_lot::Mutex::new(std::collections::HashMap::new()))
}

pub fn register_opencode_session(session_id: &str, identity: OpencodeIdentity) {
    opencode_sessions()
        .lock()
        .insert(session_id.to_string(), identity);
}

/// Resolve an OpenCode caller: by its session id when registered, else by
/// the turn in flight for `job`.
pub fn opencode_identity(session_id: Option<&str>, job: &str) -> Option<OpencodeIdentity> {
    if let Some(sid) = session_id {
        if let Some(id) = opencode_sessions().lock().get(sid) {
            return Some(id.clone());
        }
    }
    opencode_turns().lock().get(job).cloned()
}

fn begin_opencode_turn(identity: &OpencodeIdentity) {
    opencode_turns()
        .lock()
        .insert(identity.job.clone(), identity.clone());
}

fn end_opencode_turn(job: &str) {
    opencode_turns().lock().remove(job);
}

/// `opencode run` argv for one turn. A first turn creates the session
/// (`--title` names it; a JSON run reports its id on the first event, a text
/// run is looked up by title afterwards); a later turn continues it with
/// `--session`. `json` asks for `--format json`: OpenCode V1 (1.19.x) stalls
/// forever in that mode whenever an MCP server is configured (verified: even
/// a `/bin/false` server; the default format runs fine), so a turn with the
/// connector attached runs in the default format unless
/// `AGENT_CONSOLE_OPENCODE_JSON_MCP` is set (V2 users). The prompt travels as
/// the positional argument: `opencode run` takes no stdin prompt, which caps a
/// turn at the Windows 32 KiB command line (the room prompt stays well under
/// it). A model value `provider/model#variant` splits into `--model` +
/// `--variant`.
fn opencode_run_args(
    ctx: &RunCtx,
    session_id: Option<&str>,
    title: &str,
    json: bool,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["run".into()];
    if json {
        args.push("--format".into());
        args.push("json".into());
    }
    match session_id {
        Some(id) => {
            args.push("--session".into());
            args.push(id.into());
        }
        None => {
            args.push("--title".into());
            args.push(title.into());
        }
    }
    match ctx.tools {
        // OpenCode's built-in `plan` agent is read-only by design; `build`
        // with `--auto` approves its own tool calls, since nobody is at the
        // prompt.
        ToolPolicy::ReadOnly | ToolPolicy::Plan => {
            args.push("--agent".into());
            args.push("plan".into());
        }
        ToolPolicy::AcceptEdits | ToolPolicy::Full => {
            args.push("--agent".into());
            args.push("build".into());
            args.push("--auto".into());
        }
    }
    if !ctx.model.is_empty() {
        let (model, variant) = match ctx.model.split_once('#') {
            Some((m, v)) if !v.is_empty() => (m, Some(v)),
            _ => (ctx.model, None),
        };
        args.push("--model".into());
        args.push(model.into());
        if let Some(v) = variant {
            args.push("--variant".into());
            args.push(v.into());
        }
    }
    args.push(ctx.prompt.into());
    args
}

/// Whether a turn may use `--format json`: always without the connector;
/// with it only when the user vouches for a CLI that streams JSON alongside
/// MCP servers (see `opencode_run_args`).
fn opencode_json_mode(with_mcp: bool) -> bool {
    !with_mcp || std::env::var_os("AGENT_CONSOLE_OPENCODE_JSON_MCP").is_some()
}

/// Drop ANSI escape sequences from a default-format line.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// The newest session titled `title` in `cwd`, from `opencode session list
/// --format json` (`[{id, title, created, directory}]`).
fn pick_session_by_title(raw: &str, title: &str, cwd: &Path) -> Option<String> {
    let rows: Vec<Value> = serde_json::from_str(raw).ok()?;
    let here = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    rows.iter()
        .filter(|r| r.get("title").and_then(Value::as_str) == Some(title))
        .filter(|r| {
            r.get("directory")
                .and_then(Value::as_str)
                .map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| PathBuf::from(d)) == here)
                .unwrap_or(false)
        })
        .max_by_key(|r| r.get("created").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|r| r.get("id").and_then(Value::as_str))
        .map(str::to_string)
}

fn lookup_opencode_session(cwd: &Path, title: &str) -> Option<String> {
    let mut cmd =
        claude_cli::opencode_command_in(cwd, &["session", "list", "--format", "json", "-n", "50"]);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    pick_session_by_title(&String::from_utf8_lossy(&out.stdout), title, cwd)
}

/// What one `--format json` line contributes to the turn.
#[derive(Debug, Default)]
struct OpencodeTurn {
    text: String,
    tokens: u64,
    cost_usd: f64,
    /// Every `sessionID` seen; must be exactly the session we ran.
    sessions: std::collections::HashSet<String>,
    /// The last generation ended with a stop (`step_finish.reason == stop`)
    /// or a finished text part — earlier text does not prove completion.
    final_seen: bool,
    error: bool,
}

fn opencode_fold(turn: &mut OpencodeTurn, v: &Value, on_activity: &ActivitySink) {
    if let Some(sid) = v.get("sessionID").and_then(Value::as_str) {
        turn.sessions.insert(sid.to_string());
    }
    let part = v.get("part").cloned().unwrap_or(Value::Null);
    match v.get("type").and_then(Value::as_str) {
        Some("step_start") => turn.final_seen = false,
        Some("tool_use") => {
            turn.final_seen = false;
            let name = part.get("tool").and_then(Value::as_str).unwrap_or("tool");
            let state = part.get("state");
            let detail = summarize_tool_input(name, state.and_then(|s| s.get("input")));
            match state.and_then(|s| s.get("status")).and_then(Value::as_str) {
                // The same part comes back once it finished: the bash tool
                // reports its exit code in `metadata.exit` (OpenCode 1.19),
                // an errored tool carries `error`. Surface the verdict as its
                // own activity so a room shows "exit 1", not just "ran".
                Some("completed") | Some("error") => {
                    let exit = state
                        .and_then(|s| s.pointer("/metadata/exit"))
                        .and_then(Value::as_i64);
                    let errored = state.and_then(|s| s.get("status")).and_then(Value::as_str)
                        == Some("error");
                    let verdict = match (exit, errored) {
                        (Some(c), _) => format!(" · exit {c}"),
                        (None, true) => " · error".to_string(),
                        (None, false) => String::new(),
                    };
                    on_activity("tool_result", name, &format!("{detail}{verdict}"));
                }
                _ => on_activity("tool", name, &detail),
            }
        }
        Some("text") => {
            if let Some(t) = part.get("text").and_then(Value::as_str) {
                if !t.is_empty() {
                    turn.text.push_str(t);
                    on_activity("text", "", t);
                }
            }
            turn.final_seen = part.pointer("/time/end").is_some_and(|e| !e.is_null());
        }
        Some("reasoning") => {
            if let Some(t) = part.get("text").and_then(Value::as_str) {
                on_activity("thinking", "", &truncate(t, 280));
            }
        }
        Some("step_finish") => {
            turn.final_seen = part.get("reason").and_then(Value::as_str) == Some("stop");
            let tokens = part.get("tokens");
            let get = |k: &str| {
                tokens
                    .and_then(|t| t.get(k))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            turn.tokens = turn.tokens.saturating_add(get("input") + get("output"));
            turn.cost_usd += part.get("cost").and_then(Value::as_f64).unwrap_or(0.0);
        }
        Some("error") => turn.error = true,
        _ => {}
    }
}

impl EngineRunner for OpencodeRunner {
    fn run(&self, ctx: &RunCtx, on_activity: &ActivitySink) -> AppResult<TurnOutput> {
        // The connector rides on `opencode.json` of the working folder, not
        // on argv; written before the run so a fresh OpenCode picks it up.
        let identity = ctx.mcp.as_ref().map(|mcp| OpencodeIdentity {
            project: mcp.project.to_string(),
            job: mcp.job.to_string(),
            caller: mcp.caller.to_string(),
        });
        if let Some(mcp) = &ctx.mcp {
            crate::services::opencode_service::ensure_config_entry(
                ctx.cwd,
                mcp.bridge,
                mcp.project,
                mcp.job,
            )
            .map_err(|e| AppError::Other(format!("opencode connector config: {e}")))?;
        }
        if let Some(id) = &identity {
            if let Some(sid) = ctx.resume {
                register_opencode_session(sid, id.clone());
            }
            begin_opencode_turn(id);
        }
        // Unique per turn so a text-mode run can be found again by title.
        let title = format!(
            "Agent Console · {} · {}",
            ctx.mcp.as_ref().map(|m| m.job).unwrap_or("room"),
            uuid::Uuid::new_v4().simple()
        );
        let json = opencode_json_mode(ctx.mcp.is_some());
        let args = opencode_run_args(ctx, ctx.resume, &title, json);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut cmd = claude_cli::opencode_command_in(ctx.cwd, &arg_refs);
        let result = (|| {
            let mut child = cmd.spawn().map_err(|e| {
                AppError::Other(format!("failed to spawn `opencode`: {e}. Is it on PATH?"))
            })?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| AppError::Other("opencode produced no stdout pipe".into()))?;
            let err_handle = drain_stderr(child.stderr.take());
            let local = ChildSlot::default();
            let slot = park(ctx.child_slot, &local, child);

            let mut turn = OpencodeTurn::default();
            let mut text_lines: Vec<String> = Vec::new();
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if json {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    // A fresh session names itself on its first event: bind it to
                    // the participant now, so a connector call mid-turn resolves
                    // by session id and a resumed turn later finds it too.
                    if ctx.resume.is_none() && turn.sessions.is_empty() {
                        if let (Some(sid), Some(id)) =
                            (v.get("sessionID").and_then(Value::as_str), &identity)
                        {
                            register_opencode_session(sid, id.clone());
                        }
                    }
                    opencode_fold(&mut turn, &v, on_activity);
                } else {
                    // Default format: the agent's text, one line at a time —
                    // no tokens, no tool events (the V1 JSON stream is not
                    // available with MCP, see `opencode_run_args`).
                    let clean = strip_ansi(&line);
                    if std::env::var_os("AGENT_CONSOLE_OPENCODE_DEBUG").is_some() {
                        eprintln!("opencode stdout: {line:?} -> {clean:?}");
                    }
                    if !clean.trim().is_empty() {
                        on_activity("text", "", &format!("{clean}\n"));
                    }
                    text_lines.push(clean);
                }
            }
            // Keep stderr: a default-format run that says nothing on stdout
            // explains itself there (a refused model, a dead MCP server, a
            // login prompt), and the error below quotes it.
            let stderr_text = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
            finish_with_stderr(unpark(slot)?.wait(), &stderr_text, "opencode")?;
            if json {
                let session_id = match ctx.resume {
                    Some(id) => id.to_string(),
                    None => turn.sessions.iter().next().cloned().unwrap_or_default(),
                };
                let foreign = turn.sessions.iter().any(|s| s != &session_id);
                if session_id.is_empty()
                    || foreign
                    || turn.text.is_empty()
                    || !turn.final_seen
                    || turn.error
                {
                    return Err(AppError::Other(
                        "OpenCode did not complete the correct turn (no final text, an error, or another session's events)".into(),
                    ));
                }
                return Ok(TurnOutput {
                    text: turn.text,
                    session_id: Some(session_id),
                    tokens: turn.tokens,
                    cost_usd: turn.cost_usd,
                });
            }
            let text = text_lines.join("\n").trim().to_string();
            if text.is_empty() {
                return Err(AppError::Other(format!(
                    "OpenCode produced no reply (default-format run of `{}`; stderr: {})",
                    claude_cli::opencode_bin(),
                    truncate(strip_ansi(&stderr_text).trim(), 600)
                )));
            }
            let session_id = match ctx.resume {
                Some(id) => Some(id.to_string()),
                None => lookup_opencode_session(ctx.cwd, &title),
            };
            if let (Some(sid), Some(id)) = (&session_id, &identity) {
                register_opencode_session(sid, id.clone());
            }
            Ok(TurnOutput {
                text,
                session_id,
                tokens: 0,
                cost_usd: 0.0,
            })
        })();
        if let Some(id) = &identity {
            end_opencode_turn(&id.job);
        }
        result
    }
}

// ---------------- Claude ----------------

pub struct ClaudeRunner;

/// `claude -p` argv for one turn. The prompt is NOT here: it travels over
/// stdin (bare `-p` reads it), which keeps a multi-kilobyte room prompt out of
/// argv — Windows caps a command line at 32 KiB and fails the spawn with
/// os error 206 past it, the same failure #181 fixed for the scheduler,
/// advisor and learning runs.
fn claude_args(ctx: &RunCtx) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        // Stream token deltas, not just whole messages — so the staleness
        // clock can tell a healthy 40s reasoning burst from a hang. The
        // store coalesces these deltas back into one growing block.
        "--include-partial-messages".into(),
    ];
    // Empty model = the user's configured default (settings.json `model`):
    // unattended runs don't pick a model, rooms do.
    if !ctx.model.is_empty() {
        args.push("--model".into());
        args.push(ctx.model.into());
    }
    match ctx.tools {
        // No flag: headless `claude -p` allows read-style tools without
        // approval and auto-denies edits/shell (it can't prompt) — exactly
        // read-only.
        ToolPolicy::ReadOnly => {}
        ToolPolicy::AcceptEdits => {
            args.push("--permission-mode".into());
            args.push("acceptEdits".into());
        }
        ToolPolicy::Plan => {
            args.push("--permission-mode".into());
            args.push("plan".into());
        }
        ToolPolicy::Full => args.push("--dangerously-skip-permissions".into()),
    }
    if let Some(r) = ctx.resume {
        args.push("--resume".into());
        args.push(r.into());
    }
    if let Some(mcp) = &ctx.mcp {
        args.push("--mcp-config".into());
        args.push(mcp.claude_config_json());
        // Headless `-p` auto-denies any tool that would need a prompt; the
        // allow-list is what lets a read-only room turn call `delegate_task`.
        args.push("--allowedTools".into());
        args.push(format!("mcp__{MCP_SERVER_NAME}__*"));
    }
    args
}

impl EngineRunner for ClaudeRunner {
    fn run(&self, ctx: &RunCtx, on_activity: &ActivitySink) -> AppResult<TurnOutput> {
        let args = claude_args(ctx);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut cmd = claude_cli::command_with_stdin(&arg_refs);
        cmd.current_dir(ctx.cwd);
        let mut child = cmd.spawn().map_err(|e| {
            AppError::Other(format!("failed to spawn `claude`: {e}. Is it on PATH?"))
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Other("claude produced no stdout pipe".into()))?;
        let err_handle = drain_stderr(child.stderr.take());
        // Prompt over stdin; closing it is what tells `claude -p` the prompt
        // is complete.
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Other("claude produced no stdin pipe".into()))?;
        stdin
            .write_all(ctx.prompt.as_bytes())
            .map_err(|e| AppError::Other(format!("claude stdin write failed: {e}")))?;
        drop(stdin);
        let local = ChildSlot::default();
        let slot = park(ctx.child_slot, &local, child);

        let mut final_text = String::new();
        let mut session_id: Option<String> = None;
        let mut tokens: u64 = 0;
        let mut cost_usd: f64 = 0.0;

        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("system")
                    if v.get("subtype").and_then(Value::as_str) == Some("init")
                        && session_id.is_none() =>
                {
                    session_id = v
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                Some("assistant") => {
                    if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                        for block in content {
                            match block.get("type").and_then(Value::as_str) {
                                Some("thinking") => {
                                    if let Some(t) = block.get("thinking").and_then(Value::as_str) {
                                        on_activity("thinking", "", &truncate(t, 280));
                                    }
                                }
                                Some("tool_use") => {
                                    let name =
                                        block.get("name").and_then(Value::as_str).unwrap_or("tool");
                                    let detail = summarize_tool_input(name, block.get("input"));
                                    on_activity("tool", name, &detail);
                                }
                                // Final text streams token-by-token via the
                                // stream_event arm below; emitting the whole
                                // block here too would duplicate it.
                                _ => {}
                            }
                        }
                    }
                }
                Some("stream_event") => {
                    let ev = v.get("event");
                    let is_text_delta = ev.and_then(|e| e.get("type")).and_then(Value::as_str)
                        == Some("content_block_delta")
                        && ev
                            .and_then(|e| e.pointer("/delta/type"))
                            .and_then(Value::as_str)
                            == Some("text_delta");
                    if is_text_delta {
                        if let Some(t) = ev
                            .and_then(|e| e.pointer("/delta/text"))
                            .and_then(Value::as_str)
                        {
                            if !t.is_empty() {
                                on_activity("text", "", t);
                            }
                        }
                    }
                }
                Some("result") => {
                    final_text = v
                        .get("result")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
                        session_id = Some(sid.to_string());
                    }
                    cost_usd = v
                        .get("total_cost_usd")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    tokens = v.get("usage").map(claude_sum_usage).unwrap_or(0);
                }
                _ => {}
            }
        }

        finish(unpark(slot)?.wait(), err_handle, "claude")?;
        Ok(TurnOutput {
            text: final_text,
            session_id,
            tokens,
            cost_usd,
        })
    }
}

// ---------------- Codex ----------------

pub struct CodexRunner;

impl EngineRunner for CodexRunner {
    fn run(&self, ctx: &RunCtx, on_activity: &ActivitySink) -> AppResult<TurnOutput> {
        let args = codex_exec_args(ctx);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut cmd = claude_cli::codex_command_with_stdin(&arg_refs);
        cmd.current_dir(ctx.cwd);
        let mut child = cmd.spawn().map_err(|e| {
            AppError::Other(format!("failed to spawn `codex`: {e}. Is it on PATH?"))
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Other("codex produced no stdout pipe".into()))?;
        let err_handle = drain_stderr(child.stderr.take());
        // Feed the prompt over stdin (argv ends with `-`). Codex's exec mode
        // blocks until stdin is closed, so dropping the handle right after the
        // write is load-bearing, not just tidy.
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Other("codex produced no stdin pipe".into()))?;
        stdin
            .write_all(ctx.prompt.as_bytes())
            .map_err(|e| AppError::Other(format!("codex stdin write failed: {e}")))?;
        drop(stdin);
        let local = ChildSlot::default();
        let slot = park(ctx.child_slot, &local, child);

        let mut final_text = String::new();
        let mut session_id: Option<String> = None;
        let mut tokens: u64 = 0;

        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("thread.started") if session_id.is_none() => {
                    session_id = v
                        .get("thread_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                // `item.started` is Codex's only live pulse (no token deltas):
                // it fires when a tool/message begins, which is enough for the
                // staleness clock and to show "running a command" in real time.
                Some("item.started") => {
                    if let Some(item) = v.get("item") {
                        if item.get("type").and_then(Value::as_str) == Some("command_execution") {
                            let cmd_str = item.get("command").and_then(Value::as_str).unwrap_or("");
                            on_activity("tool", "shell", &truncate(cmd_str, 120));
                        }
                    }
                }
                Some("item.completed") => {
                    if let Some(item) = v.get("item") {
                        match item.get("type").and_then(Value::as_str) {
                            // Last agent_message wins as the turn's final text.
                            // Codex has no streaming deltas, so we surface the
                            // whole message as one text activity.
                            Some("agent_message") => {
                                if let Some(t) = item.get("text").and_then(Value::as_str) {
                                    final_text = t.to_string();
                                    if !t.is_empty() {
                                        on_activity("text", "", t);
                                    }
                                }
                            }
                            Some("reasoning") => {
                                if let Some(t) = item.get("text").and_then(Value::as_str) {
                                    on_activity("thinking", "", &truncate(t, 280));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some("turn.completed") => {
                    tokens = v.get("usage").map(codex_sum_usage).unwrap_or(0);
                }
                _ => {}
            }
        }

        finish(unpark(slot)?.wait(), err_handle, "codex")?;
        // Codex does not report a dollar cost — only token usage.
        Ok(TurnOutput {
            text: final_text,
            session_id,
            tokens,
            cost_usd: 0.0,
        })
    }
}

// ---------------- shared helpers ----------------

fn codex_exec_args(ctx: &RunCtx) -> Vec<String> {
    // `codex exec resume <id>` continues a prior thread (retaining its
    // memory); a fresh `codex exec` starts one. Resume has a reduced flag
    // set: it rejects -s/--sandbox/-C, inheriting the session's sandbox and
    // taking cwd from the process (set via current_dir below).
    let mut args: Vec<String> = vec!["exec".into()];
    if let Some(r) = ctx.resume {
        args.push("resume".into());
        args.push(r.into());
    }
    args.push("--json".into());
    // Run outside a git repo without complaint — kills the "needs a commit"
    // requirement entirely.
    args.push("--skip-git-repo-check".into());
    // Empty = the user's configured effort (no override).
    if !ctx.model.is_empty() {
        args.push("-c".into());
        args.push(format!("model_reasoning_effort={}", ctx.model));
    }
    match ctx.tools {
        // Full bypass mirrors Claude's skip-permissions: actually execute
        // commands instead of auto-denying them. Accepted on resume too.
        ToolPolicy::Full => args.push("--dangerously-bypass-approvals-and-sandbox".into()),
        // Sandbox (-s) is only settable on a fresh exec; on resume it is
        // inherited from the session, so we omit it there.
        policy if ctx.resume.is_none() => {
            args.push("-s".into());
            args.push(match policy {
                ToolPolicy::ReadOnly | ToolPolicy::Plan => "read-only".into(),
                _ => "workspace-write".into(),
            });
        }
        _ => {}
    }
    if let Some(mcp) = &ctx.mcp {
        args.push("-c".into());
        args.push(mcp.codex_config_override());
    }
    // Keep the large, multiline room prompt out of argv. On Windows npm shims
    // are .cmd files, and Rust rejects some batch-file arguments that cannot be
    // escaped safely. `-` asks Codex to read the prompt from stdin instead.
    args.push("-".into());
    args
}

/// Park the child where the caller can reach it (or in `local` when the
/// caller passed no slot). Returns the slot to `unpark` from.
fn park<'a>(
    shared: Option<&'a ChildSlot>,
    local: &'a ChildSlot,
    child: std::process::Child,
) -> &'a ChildSlot {
    let slot = shared.unwrap_or(local);
    *slot.lock() = Some(child);
    slot
}

/// Take the child back to wait on it. Empty means something other than the
/// runner reaped it, which the contract forbids — report rather than hang.
fn unpark(slot: &ChildSlot) -> AppResult<std::process::Child> {
    slot.lock()
        .take()
        .ok_or_else(|| AppError::Other("turn child vanished from its slot".into()))
}

/// Drain a child's stderr on its own thread so a chatty stream can't fill the
/// pipe buffer and deadlock the child.
fn drain_stderr(
    stderr: Option<std::process::ChildStderr>,
) -> Option<std::thread::JoinHandle<String>> {
    stderr.map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    })
}

/// Wait on the child and turn a non-zero exit into an error carrying stderr.
fn finish(
    status: std::io::Result<std::process::ExitStatus>,
    err_handle: Option<std::thread::JoinHandle<String>>,
    bin: &str,
) -> AppResult<()> {
    let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
    finish_with_stderr(status, &err, bin)
}

/// `finish` for a caller that already drained stderr (and wants to keep it).
fn finish_with_stderr(
    status: std::io::Result<std::process::ExitStatus>,
    err: &str,
    bin: &str,
) -> AppResult<()> {
    let status = status?;
    if !status.success() {
        let mut msg = format!(
            "{bin} exited with status {status}: {}",
            truncate(err.trim(), 600)
        );
        // Background runs (scheduler, digests) are where an expired login hurts
        // most — nobody is watching the terminal. Ask the CLI directly instead
        // of hoping its stderr names the real cause.
        if bin == "claude" {
            if let Some(hint) = crate::services::claude_cli::logged_out_hint() {
                msg.push_str(&hint);
            } else if let Some(hint) = crate::services::claude_cli::failure_hint(err) {
                // Text heuristics second: an expired login the probe missed,
                // or a configured model newer than this CLI.
                msg.push_str(&hint);
            }
        }
        return Err(AppError::Other(msg));
    }
    Ok(())
}

/// Real new tokens in a Claude turn. Excludes `cache_read_input_tokens`: a
/// resumed turn re-reads the whole cached prompt (huge yet near-free), so
/// counting it inflates the total ~10-20x and trips the budget early.
fn claude_sum_usage(u: &Value) -> u64 {
    [
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
    ]
    .iter()
    .filter_map(|k| u.get(*k).and_then(Value::as_u64))
    .sum()
}

/// Real new tokens in a Codex turn. Excludes `cached_input_tokens` for the same
/// reason Claude excludes cache reads.
fn codex_sum_usage(u: &Value) -> u64 {
    ["input_tokens", "output_tokens", "reasoning_output_tokens"]
        .iter()
        .filter_map(|k| u.get(*k).and_then(Value::as_u64))
        .sum()
}

/// A compact one-line summary of a tool call's input for the activity feed.
pub(crate) fn summarize_tool_input(name: &str, input: Option<&Value>) -> String {
    let Some(input) = input else {
        return String::new();
    };
    let pick = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
    let raw = match name {
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => pick("file_path")
            .or_else(|| pick("path"))
            .or_else(|| pick("notebook_path")),
        "Bash" => pick("command"),
        "Grep" => pick("pattern"),
        "Glob" => pick("pattern"),
        "WebFetch" => pick("url"),
        "WebSearch" => pick("query"),
        _ => None,
    };
    match raw {
        Some(s) => truncate(s.trim(), 120),
        None => truncate(input.to_string().trim_matches(&['{', '}'][..]), 100),
    }
}

/// Truncate to `max` bytes on a char boundary, appending an ellipsis.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn engine_defaults_to_claude_for_old_payloads() {
        // A participant payload that predates the `engine` field must still
        // deserialize — as Claude, the only engine that existed before.
        #[derive(Deserialize)]
        struct P {
            #[serde(default)]
            engine: Engine,
        }
        let p: P = serde_json::from_value(json!({})).unwrap();
        assert_eq!(p.engine, Engine::Claude);
        let p: P = serde_json::from_value(json!({ "engine": "codex" })).unwrap();
        assert_eq!(p.engine, Engine::Codex);
    }

    #[test]
    fn claude_usage_excludes_cache_reads() {
        let u = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_creation_input_tokens": 10,
            "cache_read_input_tokens": 99999, // huge, near-free — must be ignored
        });
        assert_eq!(claude_sum_usage(&u), 160);
    }

    #[test]
    fn codex_usage_excludes_cached_input() {
        let u = json!({
            "input_tokens": 12666,
            "cached_input_tokens": 4992, // re-read cache — must be ignored
            "output_tokens": 9,
            "reasoning_output_tokens": 3,
        });
        assert_eq!(codex_sum_usage(&u), 12678);
    }

    #[test]
    fn plan_policy_and_empty_model_map_to_the_unattended_shape() {
        let ctx = RunCtx {
            cwd: std::path::Path::new("."),
            model: "",
            tools: ToolPolicy::Plan,
            prompt: "p",
            resume: None,
            child_slot: None,
            mcp: None,
        };
        let args = claude_args(&ctx);
        assert!(args.windows(2).any(|w| w == ["--permission-mode", "plan"]));
        assert!(
            !args.iter().any(|a| a == "--model"),
            "no model ⇒ user's default"
        );
        let codex = codex_exec_args(&ctx);
        assert!(codex.windows(2).any(|w| w == ["-s", "read-only"]));
        assert!(!codex
            .iter()
            .any(|a| a.starts_with("model_reasoning_effort")));
    }

    #[test]
    fn claude_argv_carries_no_prompt_and_reads_stdin() {
        let prompt = "A room prompt\nlong enough to matter & full of | shell < chars >";
        let ctx = RunCtx {
            cwd: std::path::Path::new("."),
            model: "opus",
            tools: ToolPolicy::AcceptEdits,
            prompt,
            resume: Some("sess-1"),
            child_slot: None,
            mcp: None,
        };
        let args = claude_args(&ctx);
        assert_eq!(args.first().map(String::as_str), Some("-p"));
        // Bare `-p`: the next argument is a flag, not the prompt.
        assert_eq!(args.get(1).map(String::as_str), Some("--output-format"));
        assert!(!args.iter().any(|a| a.contains("room prompt")));
        assert!(args
            .windows(2)
            .any(|w| w == ["--permission-mode", "acceptEdits"]));
        assert!(args.windows(2).any(|w| w == ["--resume", "sess-1"]));
        assert!(args.windows(2).any(|w| w == ["--model", "opus"]));
    }

    #[cfg(unix)]
    #[test]
    fn a_parked_child_can_be_killed_from_outside_and_unpark_reports_the_signal() {
        let shared = ChildSlot::default();
        let local = ChildSlot::default();
        let child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("sleep exists on unix");
        let slot = park(Some(&shared), &local, child);
        assert!(std::ptr::eq(slot, &shared), "shared slot wins over local");
        assert!(local.lock().is_none());
        // Another thread (stop / discard / watchdog) kills it through the slot.
        kill_parked(&shared);
        kill_parked(&shared); // idempotent
        let mut child = unpark(&shared).unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success());
        assert!(unpark(&shared).is_err(), "taken once, gone after");
    }

    #[test]
    fn without_a_shared_slot_the_local_one_holds_the_child() {
        let local = ChildSlot::default();
        let child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                vec!["/C", "exit 0"]
            } else {
                vec![]
            })
            .spawn();
        let Ok(child) = child else { return };
        let slot = park(None, &local, child);
        assert!(std::ptr::eq(slot, &local));
        assert!(unpark(slot).is_ok());
    }

    fn attach<'a>() -> McpAttach<'a> {
        McpAttach {
            bridge: Path::new(r"C:\Users\Melissa Ortiz\AppData\bin\hook-bridge.exe"),
            project: "/home/u/proj",
            job: "room-42",
            caller: "p2",
        }
    }

    #[test]
    fn claude_turn_with_connector_registers_the_mcp_server_and_allows_its_tools() {
        let ctx = RunCtx {
            cwd: Path::new("."),
            model: "",
            tools: ToolPolicy::ReadOnly,
            prompt: "p",
            resume: None,
            child_slot: None,
            mcp: Some(attach()),
        };
        let args = claude_args(&ctx);
        let i = args.iter().position(|a| a == "--mcp-config").unwrap();
        let cfg: Value = serde_json::from_str(&args[i + 1]).unwrap();
        let server = &cfg["mcpServers"]["agent_console"];
        assert_eq!(
            server["command"],
            r"C:\Users\Melissa Ortiz\AppData\bin\hook-bridge.exe"
        );
        assert_eq!(
            server["args"],
            serde_json::json!([
                "mcp",
                "--project",
                "/home/u/proj",
                "--job",
                "room-42",
                "--caller",
                "p2"
            ])
        );
        assert!(args
            .windows(2)
            .any(|w| w == ["--allowedTools", "mcp__agent_console__*"]));
        // Deliberately NOT strict: the user's own MCP servers stay loaded.
        assert!(!args.iter().any(|a| a == "--strict-mcp-config"));
    }

    #[test]
    fn a_plain_turn_has_no_mcp_flags() {
        let ctx = RunCtx {
            cwd: Path::new("."),
            model: "",
            tools: ToolPolicy::ReadOnly,
            prompt: "p",
            resume: None,
            child_slot: None,
            mcp: None,
        };
        assert!(!claude_args(&ctx).iter().any(|a| a.contains("mcp")));
        assert!(!codex_exec_args(&ctx)
            .iter()
            .any(|a| a.contains("mcp_servers")));
    }

    #[test]
    fn codex_turn_with_connector_passes_a_toml_override_with_escaped_strings() {
        let ctx = RunCtx {
            cwd: Path::new("."),
            model: "",
            tools: ToolPolicy::ReadOnly,
            prompt: "p",
            resume: Some("thread-1"),
            child_slot: None,
            mcp: Some(attach()),
        };
        let args = codex_exec_args(&ctx);
        let i = args.iter().position(|a| a == "-c").unwrap();
        let over = &args[i + 1];
        assert!(over.starts_with("mcp_servers.agent_console={command="));
        // Backslashes doubled, space preserved — valid TOML basic string.
        assert!(over.contains(r#""C:\\Users\\Melissa Ortiz\\AppData\\bin\\hook-bridge.exe""#));
        assert!(over.contains(
            r#"args=["mcp","--project","/home/u/proj","--job","room-42","--caller","p2"]"#
        ));
        assert!(over.ends_with(r#"required=true,default_tools_approval_mode="approve"}"#));
        // Still reads the prompt from stdin.
        assert_eq!(args.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn opencode_run_args_map_policy_model_and_prompt() {
        let ctx = RunCtx {
            cwd: Path::new("."),
            model: "anthropic/claude-sonnet-4",
            tools: ToolPolicy::ReadOnly,
            prompt: "Read the README\nand report its title",
            resume: None,
            child_slot: None,
            mcp: None,
        };
        // A first turn creates the session by title; a later one continues it.
        let args = opencode_run_args(&ctx, None, "Agent Console · room-1", true);
        assert_eq!(args[..3], ["run", "--format", "json"]);
        assert!(args
            .windows(2)
            .any(|w| w == ["--title", "Agent Console · room-1"]));
        assert!(!args.iter().any(|a| a == "--session"));
        assert!(args.windows(2).any(|w| w == ["--agent", "plan"]));
        assert!(
            !args.iter().any(|a| a == "--auto"),
            "plan turns never auto-approve"
        );
        assert!(args
            .windows(2)
            .any(|w| w == ["--model", "anthropic/claude-sonnet-4"]));
        assert!(!args.iter().any(|a| a == "--variant"));
        assert_eq!(args.last().map(String::as_str), Some(ctx.prompt));
        let edits = RunCtx {
            tools: ToolPolicy::AcceptEdits,
            model: "openai/gpt-5#high",
            ..ctx
        };
        // Text mode (the connector attached on V1): no `--format json`.
        let args = opencode_run_args(&edits, Some("ses_1"), "ignored", false);
        assert_eq!(args[0], "run");
        assert!(!args.iter().any(|a| a == "--format"));
        assert!(args.windows(2).any(|w| w == ["--session", "ses_1"]));
        assert!(!args.iter().any(|a| a == "--title"));
        assert!(args.windows(3).any(|w| w == ["--agent", "build", "--auto"]));
        assert!(args.windows(2).any(|w| w == ["--model", "openai/gpt-5"]));
        assert!(args.windows(2).any(|w| w == ["--variant", "high"]));
        assert!(opencode_json_mode(false));
    }

    #[test]
    fn opencode_text_mode_helpers_strip_ansi_and_find_the_session_by_title() {
        assert_eq!(strip_ansi("\u{1b}[32mPONG\u{1b}[0m\r"), "PONG");
        assert_eq!(strip_ansi("plain"), "plain");
        let cwd = std::env::temp_dir();
        let here = cwd.to_string_lossy();
        let raw = serde_json::json!([
            { "id": "ses_old", "title": "Agent Console · r · a", "created": 1, "directory": here },
            { "id": "ses_new", "title": "Agent Console · r · a", "created": 2, "directory": here },
            { "id": "ses_other", "title": "Agent Console · r · a", "created": 3, "directory": "/definitely/elsewhere" },
            { "id": "ses_x", "title": "something else", "created": 4, "directory": here }
        ])
        .to_string();
        assert_eq!(
            pick_session_by_title(&raw, "Agent Console · r · a", &cwd).as_deref(),
            Some("ses_new")
        );
        assert_eq!(pick_session_by_title(&raw, "nope", &cwd), None);
        assert_eq!(pick_session_by_title("not json", "x", &cwd), None);
    }

    // Live end-to-end through the real `opencode` CLI with the connector
    // attached (the app is not running, so the tool answers "not reachable"
    // — which proves OpenCode → opencode.json → hook-bridge → loopback).
    // Ignored by default: needs opencode on PATH and spends a free-model turn.
    //   cargo test --lib engine_runner -- --ignored --nocapture opencode_runner_live
    #[test]
    #[ignore]
    fn opencode_runner_live_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "engine-runner-opencode-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/hook-bridge");
        assert!(
            bridge.is_file(),
            "build the bridge first: cargo build -p hook-bridge"
        );
        let attach = McpAttach {
            bridge: &bridge,
            project: &dir.to_string_lossy(),
            job: "room-live-test",
            caller: "p1",
        };
        let ctx = RunCtx {
            cwd: &dir,
            model: "",
            tools: ToolPolicy::ReadOnly,
            prompt: "Call the tool agent_console_list_participants once and reply with ONLY the exact text it returned, quoted verbatim.",
            resume: None,
            child_slot: None,
            mcp: Some(attach),
        };
        let seen = std::cell::RefCell::new(Vec::<String>::new());
        let sink =
            |kind: &str, _label: &str, text: &str| seen.borrow_mut().push(format!("{kind}:{text}"));
        let out = OpencodeRunner
            .run(&ctx, &sink)
            .expect("opencode turn should succeed");
        eprintln!(
            "text={:?} session={:?} activity={:?}",
            out.text,
            out.session_id,
            seen.borrow()
        );
        assert!(
            out.text.contains("not reachable") || out.text.contains("Unknown job"),
            "the connector answered through the bridge: {}",
            out.text
        );
        assert!(
            out.session_id
                .as_deref()
                .is_some_and(|s| s.starts_with("ses_")),
            "session found by title"
        );
        assert_eq!(
            opencode_identity(out.session_id.as_deref(), "room-live-test").map(|i| i.caller),
            Some("p1".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opencode_events_fold_into_text_tokens_cost_and_completion() {
        let lines = [
            r#"{"type":"step_start","sessionID":"ses_1","part":{}}"#,
            r#"{"type":"tool_use","sessionID":"ses_1","part":{"tool":"read","state":{"input":{"filePath":"README.md"}}}}"#,
            r#"{"type":"text","sessionID":"ses_1","part":{"text":"The title is ","time":{"start":1}}}"#,
            r#"{"type":"text","sessionID":"ses_1","part":{"text":"Demo.","time":{"start":1,"end":2}}}"#,
            r#"{"type":"step_finish","sessionID":"ses_1","part":{"reason":"stop","tokens":{"input":120,"output":30,"cache":{"read":500}},"cost":0.0012}}"#,
        ];
        let seen = std::cell::RefCell::new(Vec::<(String, String)>::new());
        let sink = |kind: &str, label: &str, _text: &str| {
            seen.borrow_mut().push((kind.into(), label.into()))
        };
        let mut turn = OpencodeTurn::default();
        for l in lines {
            opencode_fold(&mut turn, &serde_json::from_str(l).unwrap(), &sink);
        }
        assert_eq!(turn.text, "The title is Demo.");
        assert_eq!(turn.tokens, 150, "cache reads are not new tokens");
        assert!((turn.cost_usd - 0.0012).abs() < 1e-9);
        assert!(turn.final_seen && !turn.error);
        assert_eq!(turn.sessions.len(), 1);
        assert_eq!(seen.borrow()[0], ("tool".to_string(), "read".to_string()));
        // A new step after the text means the generation is not final.
        opencode_fold(
            &mut turn,
            &serde_json::from_str(r#"{"type":"step_start","sessionID":"ses_1","part":{}}"#)
                .unwrap(),
            &sink,
        );
        assert!(!turn.final_seen);
        // Events of another session are a failed turn; so is an error event.
        opencode_fold(
            &mut turn,
            &serde_json::from_str(r#"{"type":"error","sessionID":"ses_2","part":{}}"#).unwrap(),
            &sink,
        );
        assert!(turn.error && turn.sessions.len() == 2);
    }

    #[test]
    fn opencode_callers_resolve_by_session_then_by_the_turn_in_flight() {
        let id = OpencodeIdentity {
            project: "/p".into(),
            job: "r-oc".into(),
            caller: "p2".into(),
        };
        assert!(opencode_identity(Some("ses_x"), "r-oc").is_none());
        // Turn started, session id not yet seen: the room's turn answers.
        begin_opencode_turn(&id);
        assert_eq!(
            opencode_identity(None, "r-oc").map(|i| i.caller),
            Some("p2".into())
        );
        assert_eq!(
            opencode_identity(Some("ses_unknown"), "r-oc").map(|i| i.caller),
            Some("p2".into())
        );
        assert!(opencode_identity(None, "other-room").is_none());
        // First event named the session: bound for good, even after the turn ends.
        register_opencode_session("ses_9", id.clone());
        end_opencode_turn("r-oc");
        assert!(opencode_identity(None, "r-oc").is_none());
        assert_eq!(
            opencode_identity(Some("ses_9"), "r-oc").map(|i| i.caller),
            Some("p2".into())
        );
    }

    #[test]
    fn codex_exec_reads_prompt_from_stdin_for_fresh_turn() {
        let prompt = "Investigate this room turn\nwith symbols like & | < >";
        let ctx = RunCtx {
            cwd: std::path::Path::new("."),
            model: "high",
            tools: ToolPolicy::ReadOnly,
            prompt,
            resume: None,
            child_slot: None,
            mcp: None,
        };
        let args = codex_exec_args(&ctx);

        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert!(!args.iter().any(|arg| arg == prompt));
        assert!(args.windows(2).any(|w| w == ["-s", "read-only"]));
    }

    #[test]
    fn codex_resume_reads_prompt_from_stdin_without_sandbox_arg() {
        let prompt = "Continue the debate with a multiline prompt\nthat stays off argv.";
        let ctx = RunCtx {
            cwd: std::path::Path::new("."),
            model: "medium",
            tools: ToolPolicy::ReadOnly,
            prompt,
            resume: Some("thread-123"),
            child_slot: None,
            mcp: None,
        };
        let args = codex_exec_args(&ctx);

        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert!(!args.iter().any(|arg| arg == prompt));
        assert!(args
            .windows(3)
            .any(|w| w == ["exec", "resume", "thread-123"]));
        assert!(!args.iter().any(|arg| arg == "-s"));
    }

    // Live end-to-end: spawns the real `codex` CLI and makes a model call.
    // Ignored by default (slow, needs auth, spends tokens). Run explicitly:
    //   cargo test --lib engine_runner -- --ignored --nocapture codex_runner_live
    #[test]
    #[ignore]
    fn codex_runner_live_round_trip() {
        let dir = std::env::temp_dir().join("engine-runner-codex-test");
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = RunCtx {
            cwd: &dir,
            model: "low",
            tools: ToolPolicy::ReadOnly,
            prompt: "Reply with exactly: PONG. Nothing else.",
            resume: None,
            child_slot: None,
            mcp: None,
        };
        let activity_kinds = std::cell::RefCell::new(Vec::<String>::new());
        let sink = |kind: &str, _label: &str, _text: &str| {
            activity_kinds.borrow_mut().push(kind.to_string());
        };
        let out = CodexRunner
            .run(&ctx, &sink)
            .expect("codex turn should succeed");
        tracing::warn!(
            "text={:?} session_id={:?} tokens={} kinds={:?}",
            out.text,
            out.session_id,
            out.tokens,
            activity_kinds.borrow()
        );
        assert!(
            out.text.to_uppercase().contains("PONG"),
            "got: {:?}",
            out.text
        );
        assert!(
            out.session_id.is_some(),
            "should capture a resume thread id"
        );
        assert!(out.tokens > 0, "should report token usage");
        assert_eq!(out.cost_usd, 0.0, "codex reports no dollar cost");
    }

    #[test]
    fn truncate_is_char_boundary_safe() {
        // Cutting mid-multibyte-char must not panic.
        let s = "áéíóú-tail";
        let out = truncate(s, 5);
        assert!(out.ends_with('…'));
        assert!(s.starts_with(out.trim_end_matches('…')));
    }
}
