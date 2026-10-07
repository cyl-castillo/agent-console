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
//! | normalized   | Claude (`-p --output-format stream-json`) | Codex (`exec --json`)                   |
//! |--------------|-------------------------------------------|-----------------------------------------|
//! | resume id    | `result.session_id`                       | `thread.started.thread_id`              |
//! | final text   | `result.result`                           | `item.completed{agent_message}.text`    |
//! | tool call    | assistant `tool_use` block                | `item.started{command_execution}`       |
//! | tokens       | `result.usage` (sans cache reads)         | `turn.completed.usage` (sans cache)     |
//! | live pulse   | `text_delta` (per token)                  | `item.started` (per item — coarser)     |
//! | cost (USD)   | `result.total_cost_usd`                   | not reported → 0.0                       |

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;

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
    let status = status?;
    let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
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
            } else if let Some(hint) = crate::services::claude_cli::failure_hint(&err) {
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
