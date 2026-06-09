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

impl EngineRunner for ClaudeRunner {
    fn run(&self, ctx: &RunCtx, on_activity: &ActivitySink) -> AppResult<TurnOutput> {
        let mut args: Vec<String> = vec![
            "-p".into(),
            ctx.prompt.into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            // Stream token deltas, not just whole messages — so the staleness
            // clock can tell a healthy 40s reasoning burst from a hang. The
            // store coalesces these deltas back into one growing block.
            "--include-partial-messages".into(),
            "--model".into(),
            ctx.model.into(),
        ];
        match ctx.tools {
            // No flag: headless `claude -p` allows read-style tools without
            // approval and auto-denies edits/shell (it can't prompt) — exactly
            // read-only.
            ToolPolicy::ReadOnly => {}
            ToolPolicy::AcceptEdits => {
                args.push("--permission-mode".into());
                args.push("acceptEdits".into());
            }
            ToolPolicy::Full => args.push("--dangerously-skip-permissions".into()),
        }
        if let Some(r) = ctx.resume {
            args.push("--resume".into());
            args.push(r.into());
        }

        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut cmd = claude_cli::command(&arg_refs);
        cmd.current_dir(ctx.cwd);
        let mut child = cmd.spawn().map_err(|e| {
            AppError::Other(format!("failed to spawn `claude`: {e}. Is it on PATH?"))
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Other("claude produced no stdout pipe".into()))?;
        let err_handle = drain_stderr(child.stderr.take());

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
                        && session_id.is_none()
                    => {
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

        finish(child.wait(), err_handle, "claude")?;
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
                Some("thread.started")
                    if session_id.is_none() => {
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

        finish(child.wait(), err_handle, "codex")?;
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
    let effort = format!("model_reasoning_effort={}", ctx.model);
    let mut args: Vec<String> = vec!["exec".into()];
    if let Some(r) = ctx.resume {
        args.push("resume".into());
        args.push(r.into());
    }
    args.push("--json".into());
    // Run outside a git repo without complaint — kills the "needs a commit"
    // requirement entirely.
    args.push("--skip-git-repo-check".into());
    args.push("-c".into());
    args.push(effort);
    match ctx.tools {
        // Full bypass mirrors Claude's skip-permissions: actually execute
        // commands instead of auto-denying them. Accepted on resume too.
        ToolPolicy::Full => args.push("--dangerously-bypass-approvals-and-sandbox".into()),
        // Sandbox (-s) is only settable on a fresh exec; on resume it is
        // inherited from the session, so we omit it there.
        policy if ctx.resume.is_none() => {
            args.push("-s".into());
            args.push(match policy {
                ToolPolicy::ReadOnly => "read-only".into(),
                _ => "workspace-write".into(),
            });
        }
        _ => {}
    }
    // Keep the large, multiline room prompt out of argv. On Windows npm shims
    // are .cmd files, and Rust rejects some batch-file arguments that cannot be
    // escaped safely. `-` asks Codex to read the prompt from stdin instead.
    args.push("-".into());
    args
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
        return Err(AppError::Other(format!(
            "{bin} exited with status {status}: {}",
            truncate(err.trim(), 600)
        )));
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
    fn codex_exec_reads_prompt_from_stdin_for_fresh_turn() {
        let prompt = "Investigate this room turn\nwith symbols like & | < >";
        let ctx = RunCtx {
            cwd: std::path::Path::new("."),
            model: "high",
            tools: ToolPolicy::ReadOnly,
            prompt,
            resume: None,
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
        };
        let activity_kinds = std::cell::RefCell::new(Vec::<String>::new());
        let sink = |kind: &str, _label: &str, _text: &str| {
            activity_kinds.borrow_mut().push(kind.to_string());
        };
        let out = CodexRunner
            .run(&ctx, &sink)
            .expect("codex turn should succeed");
        eprintln!(
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
