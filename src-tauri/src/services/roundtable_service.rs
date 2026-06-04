//! Agent Room — N agents (Claude and/or Codex) plus the human hold one shared
//! conversation about a problem. The app is the conductor: it runs one headless
//! turn per participant in round-robin, feeds each agent the shared transcript
//! since it last spoke, and emits events the UI renders as a single group-chat
//! feed. The human is a first-class participant: their messages are injected
//! into the same transcript and every agent sees them on its next turn.
//!
//! Conversation-first and read-only: agents may READ the open project to ground
//! their reasoning but cannot edit it (Claude headless auto-denies edits, Codex
//! runs `-s read-only`). There is no isolation to manage, no winner to pick, no
//! diff to apply — the outcome is the conversation itself. Each agent keeps its
//! own resumed session so it retains its private reasoning across turns.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};
use crate::services::engine_runner::{self, Engine, RunCtx, ToolPolicy};

/// One conversation participant. Either an AI agent or, for the special id
/// `"human"`, the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    /// Stable key used to thread the participant's resumed session and to dedupe
    /// its own messages out of its prompt ("p1", "p2", …).
    pub id: String,
    /// Display name shown on the participant's messages (e.g. "Opus", "Codex").
    pub name: String,
    /// Which CLI backs this participant.
    #[serde(default)]
    pub engine: Engine,
    /// Claude: model alias ("opus" | "sonnet"). Codex: reasoning effort
    /// ("low" | "medium" | "high"). Validated shell-safe before launch.
    pub model: String,
    /// Optional role/lens framing for this participant ("the skeptic", "the
    /// implementer", …). Empty = a neutral collaborator.
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundtableConfig {
    /// The problem the room is working on.
    pub problem: String,
    /// Two or more agents. Round-robin order is list order.
    pub participants: Vec<Participant>,
    /// Total AI turns across the whole conversation. Hard stop.
    pub max_turns: u32,
    /// Cumulative token ceiling across all agents. 0 = no limit.
    pub token_budget: u64,
}

/// One message in the shared transcript — from an agent or the human.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub author_id: String,
    pub author_name: String,
    /// None for the human.
    pub engine: Option<Engine>,
    pub model: String,
    pub text: String,
    /// The AI turn number this message belongs to (human messages share the
    /// number of the turn they precede).
    pub turn: u32,
}

/// Emitted once per message (agent turn or human injection) over
/// `roundtable://turn`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundtableTurn {
    pub id: String,
    pub author_id: String,
    pub author_name: String,
    pub engine: Option<Engine>,
    pub model: String,
    pub text: String,
    pub turn: u32,
    /// True for the human's own messages.
    pub is_human: bool,
    /// Cumulative tokens across the conversation after this message.
    pub total_tokens: u64,
    /// Dollar cost reported by this turn (0 for Codex and for the human).
    pub cost_usd: f64,
}

/// Emitted on every lifecycle transition over `roundtable://status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundtableStatus {
    pub id: String,
    /// "running" | "paused" | "done" | "stopped" | "error"
    pub status: String,
    pub turn: u32,
    pub total_tokens: u64,
    pub message: Option<String>,
}

/// A live activity line within a turn, emitted over `roundtable://activity` as
/// it happens — so the feed shows what an agent is doing (reading, reasoning,
/// streaming text) in real time instead of a mute spinner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoundtableActivity {
    id: String,
    /// Participant id this activity belongs to.
    author_id: String,
    turn: u32,
    /// "thinking" | "tool" | "text"
    kind: String,
    label: String,
    text: String,
}

/// Live control surface for one room. Holds all state that must survive a driver
/// loop exiting, so the conversation can be continued: when the turn target is
/// reached the loop stops (status "awaiting") but the run stays alive, and
/// `continue_run` raises the target and respawns the driver from here.
struct RunControl {
    paused: AtomicBool,
    stopped: AtomicBool,
    /// Whether a driver loop is currently active (guards against double-spawn).
    driving: AtomicBool,
    /// Last completed AI turn; human messages are stamped with it so they sort
    /// sensibly in the feed.
    turn_no: AtomicU32,
    /// The driver runs until `turn_no` reaches this. Raised by `continue_run`.
    target_turns: AtomicU32,
    /// Cumulative tokens across the whole conversation (persists across continues).
    total_tokens: AtomicU64,
    /// 0 = no limit. A hard end (status "done"), unlike the soft turn target.
    token_budget: u64,
    /// The single shared conversation. The driver (agent turns) and `inject`
    /// (human turns) both append here.
    transcript: Mutex<Vec<Message>>,
    /// Per-participant resumed session id (retains its reasoning across turns
    /// and across continues).
    resume: Mutex<HashMap<String, String>>,
    /// Per-participant transcript index already folded into its prompt.
    last_seen: Mutex<HashMap<String, usize>>,
    participants: Vec<Participant>,
    problem: String,
    /// The open project the agents may read (cwd, read-only).
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

    /// Validate, register the run, and spawn the driver. `repo` is the open
    /// project the agents may read (cwd, read-only).
    pub fn start(
        &self,
        app: AppHandle,
        repo: PathBuf,
        config: RoundtableConfig,
    ) -> AppResult<String> {
        if !repo.is_dir() {
            return Err(AppError::NotADirectory(repo.display().to_string()));
        }
        if config.problem.trim().is_empty() {
            return Err(AppError::InvalidArgument("problem is empty".into()));
        }
        if config.participants.len() < 2 {
            return Err(AppError::InvalidArgument(
                "a room needs at least two participants".into(),
            ));
        }
        for p in &config.participants {
            if !is_safe_model(&p.model) {
                return Err(AppError::InvalidArgument(format!(
                    "invalid model value for {}",
                    p.name
                )));
            }
        }

        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("rt-{}-{}", std::process::id(), n);

        let control = Arc::new(RunControl {
            paused: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            driving: AtomicBool::new(true),
            turn_no: AtomicU32::new(0),
            target_turns: AtomicU32::new(config.max_turns),
            total_tokens: AtomicU64::new(0),
            token_budget: config.token_budget,
            transcript: Mutex::new(Vec::new()),
            resume: Mutex::new(HashMap::new()),
            last_seen: Mutex::new(HashMap::new()),
            participants: config.participants,
            problem: config.problem,
            repo,
        });
        self.runs.lock().unwrap().insert(id.clone(), control.clone());

        let driver_id = id.clone();
        tauri::async_runtime::spawn(async move {
            drive(app, driver_id, control).await;
        });

        Ok(id)
    }

    /// Run `extra` more turns, continuing the same conversation (transcript and
    /// per-agent sessions intact). Used after the room reaches its turn target
    /// and the human wants it to keep going. Idempotent if a driver is already
    /// active (just raises the target).
    pub fn continue_run(&self, app: &AppHandle, id: &str, extra: u32) -> AppResult<()> {
        let control = self
            .runs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("roundtable {id}")))?;
        let extra = extra.clamp(1, 60);
        // Extend from wherever we are now.
        let base = control.turn_no.load(Ordering::SeqCst);
        let new_target = (base + extra).max(control.target_turns.load(Ordering::SeqCst));
        control.target_turns.store(new_target, Ordering::SeqCst);
        control.paused.store(false, Ordering::SeqCst);
        // Spawn a fresh driver only if none is running. The CAS makes that race
        // free: whoever flips driving false->true owns the new loop.
        if control
            .driving
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let app = app.clone();
            let id = id.to_string();
            tauri::async_runtime::spawn(async move {
                drive(app, id, control).await;
            });
        }
        Ok(())
    }

    pub fn pause(&self, id: &str) -> AppResult<()> {
        self.with_run(id, |c| c.paused.store(true, Ordering::SeqCst))
    }

    pub fn resume(&self, id: &str) -> AppResult<()> {
        self.with_run(id, |c| c.paused.store(false, Ordering::SeqCst))
    }

    /// Post a human message into the shared transcript. It appears in the feed
    /// immediately and every agent sees it on its next turn.
    pub fn inject(&self, app: &AppHandle, id: &str, message: String) -> AppResult<()> {
        let text = message.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        let control = self
            .runs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("roundtable {id}")))?;
        let turn = control.turn_no.load(Ordering::SeqCst);
        let msg = Message {
            author_id: "human".into(),
            author_name: "You".into(),
            engine: None,
            model: String::new(),
            text,
            turn,
        };
        let total_tokens = {
            let mut t = control.transcript.lock().unwrap();
            t.push(msg.clone());
            // tokens are unchanged by a human message; report the running total
            // so the UI's cumulative counter stays monotonic.
            0
        };
        emit_turn(app, id, &msg, true, total_tokens, 0.0);
        Ok(())
    }

    /// Signal stop. The driver exits at the next turn boundary; the run record is
    /// kept so the finished transcript stays inspectable until `discard`.
    pub fn stop(&self, id: &str) -> AppResult<()> {
        if let Some(c) = self.runs.lock().unwrap().get(id).cloned() {
            c.stopped.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Drop a finished room. Idempotent.
    pub fn discard(&self, id: &str) -> AppResult<()> {
        if let Some(c) = self.runs.lock().unwrap().remove(id) {
            c.stopped.store(true, Ordering::SeqCst);
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

/// One terminal state of a driver loop. Carries how the loop ended so the tail
/// can emit the right status after `driving` is cleared.
enum DriveEnd {
    /// Reached the turn target — the conversation pauses for the human, who can
    /// continue it. The run stays alive.
    Awaiting,
    /// Token budget hit — a hard end.
    DoneBudget,
    /// Human stopped it.
    Stopped,
    /// A turn failed; the loop already emitted the error status.
    Errored,
}

/// The orchestration loop: round-robin over participants until the turn target,
/// a token budget, or a stop. All state lives in `control`, so the loop can exit
/// at the target and a later `continue_run` resumes exactly where it left off.
async fn drive(app: AppHandle, id: String, control: Arc<RunControl>) {
    let n = control.participants.len();
    let mut total_tokens = control.total_tokens.load(Ordering::SeqCst);
    emit_status(&app, &id, "running", control.turn_no.load(Ordering::SeqCst), total_tokens, None);

    let end = loop {
        let turn = control.turn_no.load(Ordering::SeqCst) + 1;
        if turn > control.target_turns.load(Ordering::SeqCst) {
            break DriveEnd::Awaiting;
        }
        if control.stopped.load(Ordering::SeqCst) {
            break DriveEnd::Stopped;
        }
        while control.paused.load(Ordering::SeqCst) && !control.stopped.load(Ordering::SeqCst) {
            emit_status(&app, &id, "paused", turn, total_tokens, None);
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        if control.stopped.load(Ordering::SeqCst) {
            break DriveEnd::Stopped;
        }
        control.turn_no.store(turn, Ordering::SeqCst);
        emit_status(&app, &id, "running", turn, total_tokens, None);

        let participant = control.participants[((turn - 1) as usize) % n].clone();

        // Snapshot the transcript and take everything this participant has not
        // seen yet, minus its own messages (it remembers those via its session).
        let (delta, seen_to): (Vec<Message>, usize) = {
            let t = control.transcript.lock().unwrap();
            let start = *control.last_seen.lock().unwrap().get(&participant.id).unwrap_or(&0);
            let delta = t[start..]
                .iter()
                .filter(|m| m.author_id != participant.id)
                .cloned()
                .collect();
            (delta, t.len())
        };

        let target = control.target_turns.load(Ordering::SeqCst);
        let prompt = build_room_prompt(&control.problem, &participant, &control.participants, &delta, turn, target);

        let cwd = control.repo.clone();
        let model = participant.model.clone();
        let engine = participant.engine;
        let resume_id = control.resume.lock().unwrap().get(&participant.id).cloned();
        let app_t = app.clone();
        let id_t = id.clone();
        let author_t = participant.id.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let on_activity = |kind: &str, label: &str, text: &str| {
                emit_activity(&app_t, &id_t, &author_t, turn, kind, label, text);
            };
            let ctx = RunCtx {
                cwd: &cwd,
                model: &model,
                tools: ToolPolicy::ReadOnly,
                prompt: &prompt,
                resume: resume_id.as_deref(),
            };
            engine_runner::runner_for(engine).run(&ctx, &on_activity)
        })
        .await;

        let outcome = match outcome {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                emit_status(&app, &id, "error", turn, total_tokens, Some(e.to_string()));
                break DriveEnd::Errored;
            }
            Err(e) => {
                emit_status(&app, &id, "error", turn, total_tokens, Some(format!("turn task panicked: {e}")));
                break DriveEnd::Errored;
            }
        };

        total_tokens = total_tokens.saturating_add(outcome.tokens);
        control.total_tokens.store(total_tokens, Ordering::SeqCst);
        if let Some(sid) = outcome.session_id {
            control.resume.lock().unwrap().insert(participant.id.clone(), sid);
        }

        let msg = Message {
            author_id: participant.id.clone(),
            author_name: participant.name.clone(),
            engine: Some(participant.engine),
            model: participant.model.clone(),
            text: outcome.text,
            turn,
        };
        control.transcript.lock().unwrap().push(msg.clone());
        // Advance only to what we had read (seen_to), NOT the current length:
        // anything the human injected while this turn ran sits past seen_to and
        // must surface on our next turn. Our own message is excluded by the
        // author_id filter, so it never replays.
        control.last_seen.lock().unwrap().insert(participant.id.clone(), seen_to);
        emit_turn(&app, &id, &msg, false, total_tokens, outcome.cost_usd);

        if control.token_budget > 0 && total_tokens >= control.token_budget {
            break DriveEnd::DoneBudget;
        }
    };

    // Release the driver slot BEFORE the terminal status, so a `continue_run`
    // racing the status can re-acquire and respawn cleanly.
    control.driving.store(false, Ordering::SeqCst);
    let turn = control.turn_no.load(Ordering::SeqCst);
    match end {
        DriveEnd::Awaiting => {
            // If a `continue_run` re-acquired the driver (driving flipped back to
            // true via its CAS) in the window since we released it, a fresh loop
            // is already running — don't paint a stale "awaiting" over it.
            if !control.driving.load(Ordering::SeqCst) {
                emit_status(&app, &id, "awaiting", turn, total_tokens, Some("reached the turn limit — add a message or continue".into()));
            }
        }
        DriveEnd::DoneBudget => {
            emit_status(&app, &id, "done", turn, total_tokens, Some("token budget reached".into()))
        }
        DriveEnd::Stopped => emit_status(&app, &id, "stopped", turn, total_tokens, None),
        // Error status already emitted inside the loop.
        DriveEnd::Errored => {}
    }
}

fn emit_status(app: &AppHandle, id: &str, status: &str, turn: u32, total_tokens: u64, message: Option<String>) {
    let _ = app.emit(
        "roundtable://status",
        RoundtableStatus {
            id: id.to_string(),
            status: status.to_string(),
            turn,
            total_tokens,
            message,
        },
    );
}

fn emit_turn(app: &AppHandle, id: &str, msg: &Message, is_human: bool, total_tokens: u64, cost_usd: f64) {
    let _ = app.emit(
        "roundtable://turn",
        RoundtableTurn {
            id: id.to_string(),
            author_id: msg.author_id.clone(),
            author_name: msg.author_name.clone(),
            engine: msg.engine,
            model: msg.model.clone(),
            text: msg.text.clone(),
            turn: msg.turn,
            is_human,
            total_tokens,
            cost_usd,
        },
    );
}

fn emit_activity(app: &AppHandle, id: &str, author_id: &str, turn: u32, kind: &str, label: &str, text: &str) {
    let _ = app.emit(
        "roundtable://activity",
        RoundtableActivity {
            id: id.to_string(),
            author_id: author_id.to_string(),
            turn,
            kind: kind.to_string(),
            label: label.to_string(),
            text: text.to_string(),
        },
    );
}

/// Collaborative-room framing: each agent continues a shared conversation with
/// its colleagues (and the human) toward a solution — not a debate to win.
fn build_room_prompt(
    problem: &str,
    me: &Participant,
    all: &[Participant],
    delta: &[Message],
    turn: u32,
    max_turns: u32,
) -> String {
    let role = if me.role.trim().is_empty() {
        String::new()
    } else {
        format!("\nYour role in this room: {}\n", me.role.trim())
    };

    let others = all
        .iter()
        .filter(|p| p.id != me.id)
        .map(|p| format!("{} ({})", p.name, p.model))
        .collect::<Vec<_>>()
        .join(", ");

    let convo = if delta.is_empty() {
        "No one has spoken yet. Open the discussion: frame how you see the problem and propose a first direction.".to_string()
    } else {
        let body = delta
            .iter()
            .map(|m| format!("{}:\n{}", m.author_name, engine_runner::truncate(&m.text, 4000)))
            .collect::<Vec<_>>()
            .join("\n\n");
        format!("New since you last spoke (your colleagues and the human):\n\"\"\"\n{body}\n\"\"\"")
    };

    format!(
        r#"You are **{name}** ({model}), one of several collaborators ({others}) plus a human, working together in a shared conversation to solve a real problem. You may READ the open project to ground your reasoning, but you cannot edit files — this is a discussion, not an implementation task.
{role}
The problem:
"""
{problem}
"""

This is turn {turn} of {max_turns}.

{convo}

How to contribute:
- Build on what others said. Add what's missing, sharpen what's vague, and say clearly when you disagree and why — but aim to converge on the best answer together, not to win.
- Treat the human's messages as high-priority steering.
- Ground claims in the actual code where relevant: read files before asserting how things work.
- Be concise and substantive. One strong contribution per turn beats a wall of text.
- If you believe the room has reached a good answer, say so and summarize it rather than manufacturing more discussion."#,
        name = me.name,
        model = me.model,
        others = others,
        role = role,
        problem = problem.trim(),
        turn = turn,
        max_turns = max_turns,
        convo = convo,
    )
}

fn is_safe_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 64
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}
