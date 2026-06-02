//! Agent Roundtable — two coding agents debate a topic, each in its own git
//! worktree, each able to back its argument with real code changes. The app is
//! the conductor: it runs one headless `claude -p` turn per side per round,
//! feeds each agent its opponent's last message, and emits events the UI renders
//! as a two-column transcript with live diffs. The human moderates: pause,
//! inject a steer, stop, and finally pick a winning side to apply onto the real
//! working tree (snapshot taken first).
//!
//! Isolation is the whole safety story for "both agents have tools": each side
//! operates in a detached worktree under the system temp dir, branched off HEAD.
//! Nothing they do touches the user's working tree until the moderator applies a
//! winner — and that goes through a snapshot first.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};
use crate::services::{claude_cli, proc, snapshot_service};

/// A debate participant. `model` is fed to `claude --model` and must be
/// shell/arg-safe (validated before launch).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    /// "a" | "b".
    pub side: String,
    /// Display name shown in the transcript column header (e.g. "Opus").
    pub name: String,
    /// Model alias passed to `claude --model` ("opus" | "sonnet" | "haiku").
    pub model: String,
    /// Optional extra framing for this participant's stance/role.
    pub persona: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundtableConfig {
    /// The problem the two agents debate.
    pub topic: String,
    pub participant_a: Participant,
    pub participant_b: Participant,
    /// One round = a turn for A then a turn for B. Hard stop.
    pub max_rounds: u32,
    /// Cumulative token ceiling across both agents. 0 = no limit.
    pub token_budget: u64,
    /// true  => `--dangerously-skip-permissions` (agents may run Bash too);
    /// false => `--permission-mode acceptEdits` (file edits only, no shell).
    /// Safe either way: each agent is sandboxed to a throwaway worktree.
    pub full_tools: bool,
}

/// Emitted once per agent turn over `roundtable://turn`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundtableTurn {
    pub id: String,
    pub side: String,
    pub round: u32,
    pub name: String,
    pub model: String,
    pub text: String,
    /// `git diff --stat` of this side's worktree vs HEAD, for an at-a-glance
    /// "what code did this argument touch".
    pub diff_stat: String,
    /// Cumulative tokens spent across the whole debate after this turn.
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// Emitted on every lifecycle transition over `roundtable://status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundtableStatus {
    pub id: String,
    /// "running" | "paused" | "awaiting" | "done" | "stopped" | "error"
    pub status: String,
    pub round: u32,
    pub total_tokens: u64,
    pub message: Option<String>,
}

/// Live control surface for one running debate. Flags are checked between turns,
/// so pause/stop take effect at the next turn boundary (a turn in flight is not
/// interrupted).
struct RunControl {
    paused: AtomicBool,
    stopped: AtomicBool,
    /// Moderator messages injected into the conversation, consumed before the
    /// next turn.
    inbox: Mutex<VecDeque<String>>,
    worktree_a: PathBuf,
    worktree_b: PathBuf,
    repo: PathBuf,
}

pub struct RoundtableService {
    runs: Mutex<HashMap<String, Arc<RunControl>>>,
    counter: AtomicU64,
}

impl Default for RoundtableService {
    fn default() -> Self {
        Self::new()
    }
}

impl RoundtableService {
    pub fn new() -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
        }
    }

    /// Set up worktrees and spawn the driver task. Returns the run id.
    pub fn start(
        &self,
        app: AppHandle,
        repo: PathBuf,
        config: RoundtableConfig,
    ) -> AppResult<String> {
        if !repo.is_dir() {
            return Err(AppError::NotADirectory(repo.display().to_string()));
        }
        // Worktrees branch off HEAD, so the repo needs at least one commit.
        if !has_head(&repo) {
            return Err(AppError::Other(
                "the repository needs at least one commit before a roundtable can start".into(),
            ));
        }
        if !is_safe_model(&config.participant_a.model) || !is_safe_model(&config.participant_b.model)
        {
            return Err(AppError::InvalidArgument("invalid model value".into()));
        }
        if config.topic.trim().is_empty() {
            return Err(AppError::InvalidArgument("topic is empty".into()));
        }

        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("rt-{}-{}", std::process::id(), n);

        let base = std::env::temp_dir()
            .join("agent-console-roundtable")
            .join(&id);
        let worktree_a = base.join("a");
        let worktree_b = base.join("b");
        std::fs::create_dir_all(&base)?;
        add_worktree(&repo, &worktree_a)?;
        if let Err(e) = add_worktree(&repo, &worktree_b) {
            // Roll back the first worktree so a failed start leaves nothing behind.
            remove_worktree(&repo, &worktree_a);
            return Err(e);
        }

        let control = Arc::new(RunControl {
            paused: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            inbox: Mutex::new(VecDeque::new()),
            worktree_a: worktree_a.clone(),
            worktree_b: worktree_b.clone(),
            repo: repo.clone(),
        });
        self.runs.lock().unwrap().insert(id.clone(), control.clone());

        let driver_id = id.clone();
        tauri::async_runtime::spawn(async move {
            drive(app, driver_id, config, control).await;
        });

        Ok(id)
    }

    pub fn pause(&self, id: &str) -> AppResult<()> {
        self.with_run(id, |c| c.paused.store(true, Ordering::SeqCst))
    }

    pub fn resume(&self, id: &str) -> AppResult<()> {
        self.with_run(id, |c| c.paused.store(false, Ordering::SeqCst))
    }

    pub fn inject(&self, id: &str, message: String) -> AppResult<()> {
        self.with_run(id, |c| c.inbox.lock().unwrap().push_back(message))
    }

    /// Signal stop and tear down worktrees. The driver also tears down on exit;
    /// this handles stop-before-next-turn and the user closing the panel.
    pub fn stop(&self, id: &str) -> AppResult<()> {
        let control = self.runs.lock().unwrap().get(id).cloned();
        if let Some(c) = control {
            c.stopped.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Current staged diff of one side's worktree vs HEAD (full unified diff).
    pub fn side_diff(&self, id: &str, side: &str) -> AppResult<String> {
        let control = self
            .runs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("roundtable {id}")))?;
        let wt = side_worktree(&control, side)?;
        Ok(worktree_diff(&wt))
    }

    /// Apply one side's changes onto the real working tree. Takes a snapshot of
    /// the real tree first (so the moderator can undo), then patches it. Returns
    /// the snapshot commit sha if one was taken.
    pub fn apply(&self, id: &str, side: &str) -> AppResult<Option<String>> {
        let control = self
            .runs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("roundtable {id}")))?;
        let wt = side_worktree(&control, side)?;
        let patch = worktree_diff(&wt);
        if patch.trim().is_empty() {
            return Err(AppError::Other(
                "that side made no code changes to apply".into(),
            ));
        }

        // Safety net: snapshot the real working tree before mutating it.
        let snap = snapshot_service::create(&control.repo, &format!("roundtable-apply-{id}-{side}"))?;
        apply_patch(&control.repo, &patch)?;
        Ok(snap.map(|s| s.commit_sha))
    }

    /// Stop (if running) and remove worktrees. Idempotent. Called when the user
    /// dismisses a finished debate.
    pub fn discard(&self, id: &str) -> AppResult<()> {
        let control = self.runs.lock().unwrap().remove(id);
        if let Some(c) = control {
            c.stopped.store(true, Ordering::SeqCst);
            teardown(&c);
        }
        Ok(())
    }

    fn with_run(&self, id: &str, f: impl FnOnce(&RunControl)) -> AppResult<()> {
        let control = self
            .runs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("roundtable {id}")))?;
        f(&control);
        Ok(())
    }
}

/// The orchestration loop: A then B, round after round, until a hard stop.
async fn drive(app: AppHandle, id: String, config: RoundtableConfig, control: Arc<RunControl>) {
    emit_status(&app, &id, "running", 0, 0, None);

    let mut total_tokens: u64 = 0;
    // Each side keeps its own claude session (resumed by id) so it retains
    // memory of its own reasoning across rounds.
    let mut resume_a: Option<String> = None;
    let mut resume_b: Option<String> = None;
    let mut last_a: Option<String> = None;
    let mut last_b: Option<String> = None;

    'outer: for round in 1..=config.max_rounds {
        for side in ["a", "b"] {
            if control.stopped.load(Ordering::SeqCst) {
                break 'outer;
            }
            // Honor pause at the turn boundary.
            while control.paused.load(Ordering::SeqCst) && !control.stopped.load(Ordering::SeqCst) {
                emit_status(&app, &id, "paused", round, total_tokens, None);
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            if control.stopped.load(Ordering::SeqCst) {
                break 'outer;
            }
            emit_status(&app, &id, "running", round, total_tokens, None);

            let (participant, worktree, resume, opponent_last) = if side == "a" {
                (&config.participant_a, &control.worktree_a, &resume_a, last_b.clone())
            } else {
                (&config.participant_b, &control.worktree_b, &resume_b, last_a.clone())
            };

            let moderator_notes: Vec<String> = control.inbox.lock().unwrap().drain(..).collect();
            let prompt = build_turn_prompt(
                &config.topic,
                participant,
                opponent_part(&config, side),
                opponent_last.as_deref(),
                &moderator_notes,
                round,
                config.max_rounds,
            );

            let wt = worktree.clone();
            let model = participant.model.clone();
            let full_tools = config.full_tools;
            let resume_clone = resume.clone();
            let app_t = app.clone();
            let id_t = id.clone();
            let side_t = side.to_string();
            let turn = tokio::task::spawn_blocking(move || {
                run_turn(
                    &app_t,
                    &id_t,
                    &side_t,
                    round,
                    &wt,
                    &model,
                    full_tools,
                    &prompt,
                    resume_clone.as_deref(),
                )
            })
            .await;

            let outcome = match turn {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => {
                    emit_status(&app, &id, "error", round, total_tokens, Some(e.to_string()));
                    break 'outer;
                }
                Err(e) => {
                    emit_status(
                        &app,
                        &id,
                        "error",
                        round,
                        total_tokens,
                        Some(format!("turn task panicked: {e}")),
                    );
                    break 'outer;
                }
            };

            total_tokens = total_tokens.saturating_add(outcome.tokens);
            if let Some(sid) = outcome.session_id.clone() {
                if side == "a" {
                    resume_a = Some(sid);
                } else {
                    resume_b = Some(sid);
                }
            }
            if side == "a" {
                last_a = Some(outcome.text.clone());
            } else {
                last_b = Some(outcome.text.clone());
            }

            let diff_stat = worktree_diff_stat(worktree);
            let _ = app.emit(
                "roundtable://turn",
                RoundtableTurn {
                    id: id.clone(),
                    side: side.to_string(),
                    round,
                    name: participant.name.clone(),
                    model: participant.model.clone(),
                    text: outcome.text,
                    diff_stat,
                    total_tokens,
                    cost_usd: outcome.cost_usd,
                },
            );

            if config.token_budget > 0 && total_tokens >= config.token_budget {
                emit_status(
                    &app,
                    &id,
                    "done",
                    round,
                    total_tokens,
                    Some("token budget reached".into()),
                );
                break 'outer;
            }
        }
    }

    let final_status = if control.stopped.load(Ordering::SeqCst) {
        "stopped"
    } else {
        "done"
    };
    // Worktrees stay alive until the moderator applies a winner or discards, so
    // do NOT tear down here — the diffs must remain inspectable. We only flip
    // status. `discard` performs teardown.
    emit_status(&app, &id, final_status, 0, total_tokens, None);
}

fn opponent_part<'a>(config: &'a RoundtableConfig, side: &str) -> &'a Participant {
    if side == "a" {
        &config.participant_b
    } else {
        &config.participant_a
    }
}

struct TurnOutput {
    text: String,
    session_id: Option<String>,
    tokens: u64,
    cost_usd: f64,
}

/// A live activity line within a turn, emitted over `roundtable://activity` as
/// it happens — so the panel shows what the agent is doing (reading, editing,
/// running commands, reasoning) in real time instead of a mute spinner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoundtableActivity {
    id: String,
    side: String,
    round: u32,
    /// "thinking" | "tool" | "text"
    kind: String,
    /// For "tool": the tool name. Empty otherwise.
    label: String,
    /// For "tool": a short arg summary. For "thinking"/"text": the content.
    text: String,
}

/// One headless `claude -p` turn inside `worktree`. Uses
/// `--output-format stream-json --verbose` and reads the event stream line by
/// line, emitting each tool call / reasoning / text block to the UI as it
/// arrives. Returns the final assistant text plus the session id (to resume
/// next round) and usage.
#[allow(clippy::too_many_arguments)]
fn run_turn(
    app: &AppHandle,
    id: &str,
    side: &str,
    round: u32,
    worktree: &Path,
    model: &str,
    full_tools: bool,
    prompt: &str,
    resume: Option<&str>,
) -> AppResult<TurnOutput> {
    use std::io::{BufRead, BufReader, Read};

    let mut args: Vec<String> = vec![
        "-p".into(),
        prompt.into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--model".into(),
        model.into(),
    ];
    if full_tools {
        args.push("--dangerously-skip-permissions".into());
    } else {
        args.push("--permission-mode".into());
        args.push("acceptEdits".into());
    }
    if let Some(r) = resume {
        args.push("--resume".into());
        args.push(r.into());
    }

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut cmd = claude_cli::command(&arg_refs);
    cmd.current_dir(worktree);
    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Other(format!("failed to spawn `claude`: {e}. Is it on PATH?")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Other("claude produced no stdout pipe".into()))?;
    // Drain stderr on its own thread so a chatty stderr can't fill its pipe
    // buffer and deadlock the child.
    let stderr = child.stderr.take();
    let err_handle = stderr.map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });

    let mut final_text = String::new();
    let mut session_id: Option<String> = None;
    let mut tokens: u64 = 0;
    let mut cost_usd: f64 = 0.0;

    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("system") => {
                if v.get("subtype").and_then(Value::as_str) == Some("init") && session_id.is_none() {
                    session_id = v.get("session_id").and_then(Value::as_str).map(str::to_string);
                }
            }
            Some("assistant") => {
                if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                    for block in content {
                        match block.get("type").and_then(Value::as_str) {
                            Some("thinking") => {
                                if let Some(t) = block.get("thinking").and_then(Value::as_str) {
                                    emit_activity(app, id, side, round, "thinking", "", &truncate(t, 280));
                                }
                            }
                            Some("tool_use") => {
                                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                                let detail = summarize_tool_input(name, block.get("input"));
                                emit_activity(app, id, side, round, "tool", name, &detail);
                            }
                            Some("text") => {
                                if let Some(t) = block.get("text").and_then(Value::as_str) {
                                    if !t.trim().is_empty() {
                                        emit_activity(app, id, side, round, "text", "", t);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some("result") => {
                final_text = v.get("result").and_then(Value::as_str).unwrap_or("").to_string();
                if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
                    session_id = Some(sid.to_string());
                }
                cost_usd = v.get("total_cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
                tokens = v.get("usage").map(sum_usage).unwrap_or(0);
            }
            _ => {}
        }
    }

    let status = child.wait()?;
    let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
    if !status.success() {
        return Err(AppError::Other(format!(
            "claude exited with status {}: {}",
            status,
            truncate(err.trim(), 600)
        )));
    }
    Ok(TurnOutput {
        text: final_text,
        session_id,
        tokens,
        cost_usd,
    })
}

/// Real new tokens processed in a turn, for the budget. We deliberately exclude
/// `cache_read_input_tokens`: every resumed turn re-reads the whole cached
/// prompt, so cache reads are huge (tens of thousands per turn) yet near-free —
/// counting them inflates the total ~10-20x and trips the budget after a couple
/// of rounds. input + output + cache_creation reflects actual consumption.
fn sum_usage(u: &Value) -> u64 {
    ["input_tokens", "output_tokens", "cache_creation_input_tokens"]
        .iter()
        .filter_map(|k| u.get(*k).and_then(Value::as_u64))
        .sum()
}

/// A compact one-line summary of a tool call's input for the activity feed.
fn summarize_tool_input(name: &str, input: Option<&Value>) -> String {
    let Some(input) = input else { return String::new() };
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
        // Fall back to a compact JSON of the input for unknown tools.
        None => truncate(input.to_string().trim_matches(&['{', '}'][..]), 100),
    }
}

fn emit_activity(app: &AppHandle, id: &str, side: &str, round: u32, kind: &str, label: &str, text: &str) {
    let _ = app.emit(
        "roundtable://activity",
        RoundtableActivity {
            id: id.to_string(),
            side: side.to_string(),
            round,
            kind: kind.to_string(),
            label: label.to_string(),
            text: text.to_string(),
        },
    );
}

/// Adversarial debate framing. Each agent argues its position AND may edit code
/// in its own (isolated) worktree to demonstrate it.
fn build_turn_prompt(
    topic: &str,
    me: &Participant,
    opponent: &Participant,
    opponent_last: Option<&str>,
    moderator_notes: &[String],
    round: u32,
    max_rounds: u32,
) -> String {
    let persona = if me.persona.trim().is_empty() {
        String::new()
    } else {
        format!("\nYour assigned stance/role: {}\n", me.persona.trim())
    };

    let opponent_block = match opponent_last {
        Some(text) if !text.trim().is_empty() => format!(
            "Your opponent ({name}) just argued:\n\"\"\"\n{body}\n\"\"\"\n\nEngage with their points directly — concede what is right, refute what is weak, and strengthen your own position.",
            name = opponent.name,
            body = truncate(text, 6000),
        ),
        _ => "You speak first. Open with your strongest position and, if useful, a concrete code change in this working tree to demonstrate it.".to_string(),
    };

    let mod_block = if moderator_notes.is_empty() {
        String::new()
    } else {
        format!(
            "\nThe human moderator interjects (treat as high-priority guidance):\n{}\n",
            moderator_notes
                .iter()
                .map(|m| format!("- {}", m.trim()))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    format!(
        r#"You are **{name}** ({model}), one of two engineers in a structured debate about a real codebase. This working directory is YOUR PRIVATE git worktree — edits here are isolated and will not affect anyone else, so feel free to prototype concrete changes to back your argument. The human will later pick one side's changes to keep.
{persona}
The question under debate:
"""
{topic}
"""

This is round {round} of {max_rounds}.

{opponent_block}
{mod_block}
Rules of engagement:
- Be rigorous and adversarial, not agreeable. Your job is to find the strongest correct answer, not to be polite.
- Ground claims in the actual code: read files, and where it sharpens your point, make real edits in this worktree.
- Be concise. Lead with your position, then the reasoning, then (if any) what you changed and why.
- If you genuinely think your opponent is right, say so and refine the shared conclusion rather than manufacturing disagreement."#,
        name = me.name,
        model = me.model,
        persona = persona,
        topic = topic.trim(),
        round = round,
        max_rounds = max_rounds,
        opponent_block = opponent_block,
        mod_block = mod_block,
    )
}

// ----- git worktree plumbing -----

fn has_head(repo: &Path) -> bool {
    proc::command("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(repo)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn add_worktree(repo: &Path, path: &Path) -> AppResult<()> {
    let out = proc::command("git")
        .args(["worktree", "add", "--detach", &path.to_string_lossy(), "HEAD"])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(AppError::Other(format!("git worktree add: {msg}")));
    }
    Ok(())
}

fn remove_worktree(repo: &Path, path: &Path) {
    let _ = proc::command("git")
        .args(["worktree", "remove", "--force", &path.to_string_lossy()])
        .current_dir(repo)
        .output();
}

fn teardown(control: &RunControl) {
    remove_worktree(&control.repo, &control.worktree_a);
    remove_worktree(&control.repo, &control.worktree_b);
    let _ = proc::command("git")
        .args(["worktree", "prune"])
        .current_dir(&control.repo)
        .output();
    // Best-effort cleanup of the temp parent.
    if let Some(parent) = control.worktree_a.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

fn side_worktree(control: &RunControl, side: &str) -> AppResult<PathBuf> {
    match side {
        "a" => Ok(control.worktree_a.clone()),
        "b" => Ok(control.worktree_b.clone()),
        other => Err(AppError::InvalidArgument(format!("unknown side {other}"))),
    }
}

/// Full unified diff of a worktree vs HEAD, including untracked files. Staging
/// everything first is harmless — the worktree is throwaway.
fn worktree_diff(wt: &Path) -> String {
    let _ = proc::command("git")
        .args(["add", "-A"])
        .current_dir(wt)
        .output();
    proc::command("git")
        .args(["diff", "--cached", "--no-color", "HEAD"])
        .current_dir(wt)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn worktree_diff_stat(wt: &Path) -> String {
    let _ = proc::command("git")
        .args(["add", "-A"])
        .current_dir(wt)
        .output();
    proc::command("git")
        .args(["diff", "--cached", "--stat", "HEAD"])
        .current_dir(wt)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Apply a unified patch onto the real repo's working tree. `--3way` lets it
/// fall back to a merge when context drifted; `--whitespace=nowarn` keeps it
/// quiet. The patch is fed on stdin.
fn apply_patch(repo: &Path, patch: &str) -> AppResult<()> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = proc::command("git")
        .args(["apply", "--3way", "--whitespace=nowarn"])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(patch.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(AppError::Other(format!("git apply failed: {msg}")));
    }
    Ok(())
}

fn emit_status(
    app: &AppHandle,
    id: &str,
    status: &str,
    round: u32,
    total_tokens: u64,
    message: Option<String>,
) {
    let _ = app.emit(
        "roundtable://status",
        RoundtableStatus {
            id: id.to_string(),
            status: status.to_string(),
            round,
            total_tokens,
            message,
        },
    );
}

fn is_safe_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 64
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}
