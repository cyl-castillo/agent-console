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

/// Live control surface for one running room. Flags are checked between turns,
/// so pause/stop take effect at the next turn boundary.
struct RunControl {
    paused: AtomicBool,
    stopped: AtomicBool,
    /// The AI turn currently in flight (or just finished); human messages are
    /// stamped with it so they sort sensibly in the feed.
    turn_no: AtomicU32,
    /// The single shared conversation. Both the driver (agent turns) and
    /// `inject` (human turns) append here.
    transcript: Mutex<Vec<Message>>,
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
            turn_no: AtomicU32::new(0),
            transcript: Mutex::new(Vec::new()),
        });
        self.runs.lock().unwrap().insert(id.clone(), control.clone());

        let driver_id = id.clone();
        tauri::async_runtime::spawn(async move {
            drive(app, driver_id, repo, config, control).await;
        });

        Ok(id)
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

/// The orchestration loop: round-robin over participants until `max_turns`, a
/// token budget, or a stop.
async fn drive(
    app: AppHandle,
    id: String,
    repo: PathBuf,
    config: RoundtableConfig,
    control: Arc<RunControl>,
) {
    emit_status(&app, &id, "running", 0, 0, None);

    let participants = config.participants;
    let n = participants.len();
    let mut total_tokens: u64 = 0;
    let mut terminal_emitted = false;
    // Per-participant resumed session id (retains its own reasoning) and the
    // transcript index it has already seen (so each prompt carries only what is
    // new since that participant last spoke).
    let mut resume: HashMap<String, String> = HashMap::new();
    let mut last_seen: HashMap<String, usize> = HashMap::new();

    'outer: for turn in 1..=config.max_turns {
        let participant = &participants[((turn - 1) as usize) % n];
        control.turn_no.store(turn, Ordering::SeqCst);

        if control.stopped.load(Ordering::SeqCst) {
            break 'outer;
        }
        while control.paused.load(Ordering::SeqCst) && !control.stopped.load(Ordering::SeqCst) {
            emit_status(&app, &id, "paused", turn, total_tokens, None);
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        if control.stopped.load(Ordering::SeqCst) {
            break 'outer;
        }
        emit_status(&app, &id, "running", turn, total_tokens, None);

        // Snapshot the transcript and take everything this participant has not
        // seen yet, minus its own messages (it remembers those via its session).
        let (delta, seen_to): (Vec<Message>, usize) = {
            let t = control.transcript.lock().unwrap();
            let start = *last_seen.get(&participant.id).unwrap_or(&0);
            let delta = t[start..]
                .iter()
                .filter(|m| m.author_id != participant.id)
                .cloned()
                .collect();
            (delta, t.len())
        };

        let prompt = build_room_prompt(&config.problem, participant, &participants, &delta, turn, config.max_turns);

        let cwd = repo.clone();
        let model = participant.model.clone();
        let engine = participant.engine;
        let resume_id = resume.get(&participant.id).cloned();
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
                terminal_emitted = true;
                break 'outer;
            }
            Err(e) => {
                emit_status(&app, &id, "error", turn, total_tokens, Some(format!("turn task panicked: {e}")));
                terminal_emitted = true;
                break 'outer;
            }
        };

        total_tokens = total_tokens.saturating_add(outcome.tokens);
        if let Some(sid) = outcome.session_id {
            resume.insert(participant.id.clone(), sid);
        }

        // Append this turn to the shared transcript and advance this
        // participant's seen marker past its own message.
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
        last_seen.insert(participant.id.clone(), seen_to);
        emit_turn(&app, &id, &msg, false, total_tokens, outcome.cost_usd);

        if config.token_budget > 0 && total_tokens >= config.token_budget {
            emit_status(&app, &id, "done", turn, total_tokens, Some("token budget reached".into()));
            terminal_emitted = true;
            break 'outer;
        }
    }

    if !terminal_emitted {
        let final_status = if control.stopped.load(Ordering::SeqCst) {
            "stopped"
        } else {
            "done"
        };
        emit_status(&app, &id, final_status, control.turn_no.load(Ordering::SeqCst), total_tokens, None);
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
