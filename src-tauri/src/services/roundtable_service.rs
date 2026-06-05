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
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};
use crate::services::engine_runner::{self, Engine, RunCtx, ToolPolicy};
use crate::services::proc;

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
    /// "Working room": agents may edit the code in an isolated worktree
    /// (`ToolPolicy::AcceptEdits`), each turn auto-committed on a `room/<id>`
    /// branch the human reviews and merges. Off (default) = conversation-only,
    /// read-only. Defaulted so existing/persisted configs still deserialize.
    #[serde(default)]
    pub allow_edits: bool,
}

/// One message in the shared transcript — from an agent or the human.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Cumulative token ceiling; 0 = no limit. A soft checkpoint, not a wall: when
    /// reached the room pauses (status "awaiting") and `continue_run` raises it, so
    /// the human can keep going. Atomic so continue can bump it at runtime.
    token_budget: AtomicU64,
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
    /// The open project root. Agents read it (conversation rooms) and it's the
    /// base a working room's worktree branches off.
    repo: PathBuf,
    /// Where each turn actually runs: `repo` for a conversation room, the
    /// isolated worktree for a working room.
    workspace: PathBuf,
    /// Permission level for every turn — `ReadOnly` or (working room) `AcceptEdits`.
    /// Never `Full`.
    tools: ToolPolicy,
    /// `Some` for a working room: the checkout the agents edit. Torn down on
    /// discard (the `branch` keeps the commits).
    worktree: Option<PathBuf>,
    /// `Some` for a working room: the `room/<id>` branch the turns commit onto,
    /// for the human to review and merge. Consumed by the review/merge UI (W2).
    #[allow(dead_code)]
    branch: Option<String>,
    /// Shared disk store for crash-safe autosave of this room (see `autosave`).
    rooms: Arc<RoomsStore>,
    /// One-time room banner emitted when the driver starts — e.g. editing was
    /// downgraded to read-only because the project isn't a git repo. `None` for
    /// the normal case.
    notice: Option<String>,
}

impl RunControl {
    /// Snapshot the live room into its on-disk form. Each inner `Mutex` is held
    /// only long enough to clone, so the caller can write to disk without holding
    /// any of this room's locks (and never serializes while a turn is mutating).
    fn snapshot(&self, id: &str) -> PersistedRoom {
        let transcript = self.transcript.lock().unwrap().clone();
        let resume = self.resume.lock().unwrap().clone();
        let last_seen = self.last_seen.lock().unwrap().clone();
        PersistedRoom {
            version: ROOM_SCHEMA_VERSION,
            id: id.to_string(),
            problem: self.problem.clone(),
            participants: self.participants.clone(),
            transcript,
            resume,
            last_seen,
            total_tokens: self.total_tokens.load(Ordering::SeqCst),
            updated_at_ms: now_ms(),
        }
    }
}

/// Persist the room's current state to `rooms.json` (best-effort). Snapshots
/// under the room's inner locks, releases them, then writes outside all of them.
/// A failed write is logged, not propagated — a turn must not die because the
/// disk hiccuped; the next `transcript.push` will try again.
fn autosave(control: &RunControl, id: &str) {
    let room = control.snapshot(id);
    if let Err(e) = control.rooms.save_room(&room, &control.repo) {
        eprintln!("roundtable: failed to persist room {id}: {e}");
    }
}

/// Bump when the on-disk shape changes incompatibly so loaders can migrate.
const ROOM_SCHEMA_VERSION: u32 = 1;
/// Keep at most this many rooms per project; the oldest are pruned on write.
const MAX_ROOMS_PER_PROJECT: usize = 50;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// On-disk form of a room: everything needed to re-hydrate the sidebar entry and
/// (Fase B) reconstruct a `RunControl`. `resume` holds the engines' opaque resume
/// references, which are session handles, not secrets — safe to persist.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedRoom {
    pub version: u32,
    pub id: String,
    pub problem: String,
    pub participants: Vec<Participant>,
    pub transcript: Vec<Message>,
    #[serde(default)]
    pub resume: HashMap<String, String>,
    #[serde(default)]
    pub last_seen: HashMap<String, usize>,
    #[serde(default)]
    pub total_tokens: u64,
    /// millis since epoch of the last write — drives sidebar ordering and retention.
    pub updated_at_ms: u64,
}

/// Lightweight sidebar entry — everything the room list shows without paying to
/// serialize every room's full transcript on each refresh. Full state is fetched
/// per-room via [`RoomsStore::get`] only when one is opened.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSummary {
    pub id: String,
    pub problem: String,
    pub participant_names: Vec<String>,
    pub message_count: usize,
    /// Highest turn number reached in the transcript (0 if empty).
    pub last_turn: u32,
    pub total_tokens: u64,
    pub updated_at_ms: u64,
}

impl RoomSummary {
    fn of(room: &PersistedRoom) -> Self {
        Self {
            id: room.id.clone(),
            problem: room.problem.clone(),
            participant_names: room.participants.iter().map(|p| p.name.clone()).collect(),
            message_count: room.transcript.len(),
            last_turn: room.transcript.iter().map(|m| m.turn).max().unwrap_or(0),
            total_tokens: room.total_tokens,
            updated_at_ms: room.updated_at_ms,
        }
    }
}

/// Outcome of sharing a working room's branch with collaborators. The push is
/// the contract; `pr_url` is a best-effort convenience for the recognized hosts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareResult {
    /// The pushed branch (e.g. "room/r-…").
    pub branch: String,
    /// The remote it was pushed to (e.g. "origin").
    pub remote: String,
    /// A ready-to-open URL where a colleague opens the MR/PR for this branch,
    /// derived from the remote host (GitHub / GitLab). `None` for an unrecognized
    /// host — the branch is still pushed and reviewable, just open the MR by hand.
    pub pr_url: Option<String>,
    /// Human-readable summary the feed shows after a share.
    pub message: String,
}

/// Outcome of syncing a colleague's commits *into* a live working room — the
/// return path of cowork (the mirror of `ShareResult`). The merge is the
/// contract; `conflicts` is non-empty exactly when the merge was aborted and
/// needs human resolution.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    /// The room branch that was synced (e.g. "room/r-…").
    pub branch: String,
    /// The remote it was fetched from (e.g. "origin").
    pub remote: String,
    /// How many of the colleague's commits were merged in (0 if already up to
    /// date or if the merge was aborted on conflict).
    pub merged_commits: usize,
    /// Files that conflicted. Empty on a clean sync; non-empty means the merge
    /// was aborted and the human must resolve these by hand.
    pub conflicts: Vec<String>,
    /// Human-readable summary the feed shows after a sync.
    pub message: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RoomsFile {
    #[serde(default)]
    by_project: HashMap<String, Vec<PersistedRoom>>,
}

/// Crash-safe, per-project JSON store for rooms — the same atomic write + `.bak`
/// recovery discipline as `SessionsService`. One instance is shared (via `Arc`)
/// by the service and every live `RunControl`; its `lock` serializes disk writes
/// across rooms autosaving concurrently.
pub struct RoomsStore {
    lock: Mutex<()>,
}

impl Default for RoomsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomsStore {
    pub fn new() -> Self {
        Self { lock: Mutex::new(()) }
    }

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("rooms.json"))
    }

    fn bak_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("rooms.json.bak"))
    }

    fn tmp_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("rooms.json.tmp"))
    }

    /// Load the rooms file. A missing/empty file is a legitimate empty state; a
    /// read or parse failure on an EXISTING file is an error so a blind save can
    /// never clobber unreadable history. On a parse failure we first try `.bak`.
    fn load_file() -> AppResult<RoomsFile> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(RoomsFile::default());
        }
        let txt = fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("read rooms.json: {e}")))?;
        if txt.trim().is_empty() {
            return Ok(RoomsFile::default());
        }
        match serde_json::from_str::<RoomsFile>(&txt) {
            Ok(file) => Ok(file),
            Err(e) => {
                if let Ok(bak) = Self::bak_path() {
                    if let Ok(btxt) = fs::read_to_string(&bak) {
                        if let Ok(file) = serde_json::from_str::<RoomsFile>(&btxt) {
                            return Ok(file);
                        }
                    }
                }
                Err(AppError::Other(format!("parse rooms.json: {e}")))
            }
        }
    }

    /// Write atomically: serialize to a temp file, back up the current good file,
    /// then rename the temp over the target. A crash mid-write can only damage
    /// the temp file, never the live rooms.json.
    fn write_file(file: &RoomsFile) -> AppResult<()> {
        let path = Self::path()?;
        let json = serde_json::to_string_pretty(file)
            .map_err(|e| AppError::Other(format!("serialize: {e}")))?;
        let tmp = Self::tmp_path()?;
        fs::write(&tmp, json.as_bytes())?;
        if path.exists() {
            if let Ok(bak) = Self::bak_path() {
                let _ = fs::copy(&path, &bak);
            }
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Upsert one room under its project, prune to the most-recent
    /// `MAX_ROOMS_PER_PROJECT`, and write atomically.
    pub fn save_room(&self, room: &PersistedRoom, project_root: &Path) -> AppResult<()> {
        let _g = self.lock.lock().unwrap();
        let mut file = Self::load_file()?;
        let key = project_root.display().to_string();
        let list = file.by_project.entry(key).or_default();
        match list.iter_mut().find(|r| r.id == room.id) {
            Some(existing) => *existing = room.clone(),
            None => list.push(room.clone()),
        }
        if list.len() > MAX_ROOMS_PER_PROJECT {
            list.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
            list.truncate(MAX_ROOMS_PER_PROJECT);
        }
        Self::write_file(&file)
    }

    /// All persisted rooms for a project, most-recently-updated first.
    fn load_sorted(project_root: &str) -> AppResult<Vec<PersistedRoom>> {
        let file = Self::load_file()?;
        let mut rooms = file.by_project.get(project_root).cloned().unwrap_or_default();
        rooms.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
        Ok(rooms)
    }

    /// Lightweight summaries for the sidebar, most-recently-updated first.
    pub fn summaries(&self, project_root: &str) -> AppResult<Vec<RoomSummary>> {
        let _g = self.lock.lock().unwrap();
        Ok(Self::load_sorted(project_root)?.iter().map(RoomSummary::of).collect())
    }

    /// The full persisted state of one room, for read-only re-hydration.
    pub fn get(&self, project_root: &str, room_id: &str) -> AppResult<Option<PersistedRoom>> {
        let _g = self.lock.lock().unwrap();
        Ok(Self::load_sorted(project_root)?.into_iter().find(|r| r.id == room_id))
    }

    /// Drop one room from a project's history. Idempotent.
    pub fn delete_room(&self, project_root: &str, room_id: &str) -> AppResult<()> {
        let _g = self.lock.lock().unwrap();
        let mut file = Self::load_file()?;
        if let Some(list) = file.by_project.get_mut(project_root) {
            list.retain(|r| r.id != room_id);
            if list.is_empty() {
                file.by_project.remove(project_root);
            }
        }
        Self::write_file(&file)
    }
}

pub struct RoundtableService {
    runs: Mutex<HashMap<String, Arc<RunControl>>>,
    rooms: Arc<RoomsStore>,
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
            rooms: Arc::new(RoomsStore::new()),
        }
    }

    /// The shared rooms store, for the IPC commands that list/open/delete
    /// persisted rooms (the live `runs` map only holds rooms from this session).
    pub fn rooms(&self) -> Arc<RoomsStore> {
        self.rooms.clone()
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

        // Stable room id: a millis timestamp (survives restarts and sorts the
        // sidebar chronologically) plus a uuid suffix for collision-freedom. The
        // old `rt-{pid}-{n}` reset its counter on every launch and embedded a pid
        // that does not survive a restart — unusable as a persisted identity.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let id = format!("r-{}-{}", ts, &uuid::Uuid::new_v4().simple().to_string()[..8]);

        // Working room: stand up an isolated worktree branched off HEAD so agents
        // edit there with `AcceptEdits` (never the user's real checkout), each
        // turn committed onto `room/<id>` for the human to review and merge. A
        // conversation room runs read-only in the project root itself.
        let mut notice: Option<String> = None;
        let (workspace, worktree, branch, tools) = if config.allow_edits {
            let branch = format!("room/{id}");
            let wt = room_worktree_path(&id);
            match add_room_worktree(&repo, &wt, &branch) {
                Ok(()) => (wt.clone(), Some(wt), Some(branch), ToolPolicy::AcceptEdits),
                // No usable git repo (or no commits) — don't kill the room. Degrade
                // to a read-only conversation so a non-git workspace (someone using
                // the app only for Jira/GitLab/MCP) still works; the human just
                // can't have agents edit without a repo to branch and review against.
                Err(_) => {
                    notice = Some(
                        "Editing needs a git repo with at least one commit — running read-only."
                            .into(),
                    );
                    (repo.clone(), None, None, ToolPolicy::ReadOnly)
                }
            }
        } else {
            (repo.clone(), None, None, ToolPolicy::ReadOnly)
        };

        let control = Arc::new(RunControl {
            paused: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            driving: AtomicBool::new(true),
            turn_no: AtomicU32::new(0),
            target_turns: AtomicU32::new(config.max_turns),
            total_tokens: AtomicU64::new(0),
            token_budget: AtomicU64::new(config.token_budget),
            transcript: Mutex::new(Vec::new()),
            resume: Mutex::new(HashMap::new()),
            last_seen: Mutex::new(HashMap::new()),
            participants: config.participants,
            problem: config.problem,
            repo,
            workspace,
            tools,
            worktree,
            branch,
            rooms: self.rooms.clone(),
            notice,
        });
        self.runs.lock().unwrap().insert(id.clone(), control.clone());

        let driver_id = id.clone();
        tauri::async_runtime::spawn(async move {
            drive(app, driver_id, control).await;
        });

        Ok(id)
    }

    /// Rebuild a live run from a persisted room (Fase B) so a saved conversation
    /// can be continued. Reuses the persisted id, so the rebuilt run autosaves
    /// back over the SAME `rooms.json` entry rather than forking a copy. Lands in
    /// the "awaiting" state with NO driver spawned (turn target == last turn):
    /// the human then adds a message and/or hits continue, which raises the
    /// target and starts the driver exactly as for a room that hit its turn
    /// limit. Best-effort — each agent's persisted resume id may have expired, in
    /// which case its next turn simply starts a fresh engine session.
    ///
    /// Idempotent within a session: if the id is already live (restored or never
    /// closed) the existing run is kept untouched.
    pub fn restore(&self, repo: PathBuf, room: PersistedRoom) -> AppResult<String> {
        if !repo.is_dir() {
            return Err(AppError::NotADirectory(repo.display().to_string()));
        }
        if room.participants.len() < 2 {
            return Err(AppError::InvalidArgument(
                "a room needs at least two participants".into(),
            ));
        }
        for p in &room.participants {
            if !is_safe_model(&p.model) {
                return Err(AppError::InvalidArgument(format!(
                    "invalid model value for {}",
                    p.name
                )));
            }
        }

        let id = room.id.clone();
        if self.runs.lock().unwrap().contains_key(&id) {
            return Ok(id);
        }

        // Resume where the saved transcript left off; `continue_run` raises the
        // target above this to actually run more turns.
        let last_turn = room.transcript.iter().map(|m| m.turn).max().unwrap_or(0);
        let control = Arc::new(RunControl {
            paused: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            driving: AtomicBool::new(false),
            turn_no: AtomicU32::new(last_turn),
            target_turns: AtomicU32::new(last_turn),
            total_tokens: AtomicU64::new(room.total_tokens),
            // The turn budget isn't persisted; a continued room runs unbounded by
            // tokens (the soft turn target still gates each round).
            token_budget: AtomicU64::new(0),
            transcript: Mutex::new(room.transcript),
            resume: Mutex::new(room.resume),
            last_seen: Mutex::new(room.last_seen),
            participants: room.participants,
            problem: room.problem,
            // A resumed room comes back conversation-only/read-only for now;
            // reattaching a working room's worktree is a later phase (the
            // `room/<id>` branch with its commits still lives in the repo).
            workspace: repo.clone(),
            tools: ToolPolicy::ReadOnly,
            worktree: None,
            branch: None,
            repo,
            rooms: self.rooms.clone(),
            notice: None,
        });
        self.runs.lock().unwrap().insert(id.clone(), control);
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
        // If we're continuing past a token-budget checkpoint, grant another full
        // window so the very next turn doesn't immediately re-trip it. Each
        // continue adds one budget's worth of headroom — the safety rail stays, it
        // never silently goes unlimited.
        let budget = control.token_budget.load(Ordering::SeqCst);
        let total = control.total_tokens.load(Ordering::SeqCst);
        if budget > 0 && total >= budget {
            control.token_budget.store(total.saturating_add(budget), Ordering::SeqCst);
        }
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
        autosave(&control, id);
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

    /// Drop a finished room. Idempotent. A working room's worktree CHECKOUT is
    /// torn down here (no orphaned dirs left in temp), but its `room/<id>` branch
    /// — and so every per-turn commit — stays in the repo for the human to merge
    /// or delete later.
    pub fn discard(&self, id: &str) -> AppResult<()> {
        if let Some(c) = self.runs.lock().unwrap().remove(id) {
            c.stopped.store(true, Ordering::SeqCst);
            if let Some(wt) = &c.worktree {
                remove_room_worktree(&c.repo, wt);
            }
        }
        Ok(())
    }

    /// Push a working room's `room/<id>` branch to the shared remote so human
    /// colleagues can review it and open an MR/PR — turning the room's per-turn
    /// commits into reviewable work in the platform the team already uses. This
    /// is the simplest cowork bridge: no realtime infra, just the branch the
    /// room is already producing, made visible to everyone on the remote.
    pub fn share(&self, id: &str) -> AppResult<ShareResult> {
        let control = self
            .runs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("roundtable {id}")))?;
        // A live working room carries its branch handle. A *resumed* room lost it
        // (reattach is W3, pending) but its `room/<id>` branch still lives in the
        // repo — fall back to it by name so a room reopened in a later session can
        // still go out for review. A genuine conversation room has no such branch,
        // so the lookup fails and we return the read-only error as before.
        let branch = match control.branch.clone() {
            Some(b) => b,
            None => {
                let candidate = format!("room/{id}");
                if branch_exists(&control.repo, &candidate) {
                    candidate
                } else {
                    return Err(AppError::Other(
                        "this is a conversation room (read-only) — only a working \
                         room produces a branch to share"
                            .into(),
                    ));
                }
            }
        };
        // Before pushing, drop the room's conversation into the branch as a
        // `.room/<id>.md` artifact and commit it. The MR then carries the full
        // reasoning next to the diff — a reviewer sees WHY each change was made,
        // asynchronously, with zero realtime infra. Best-effort: a write/commit
        // hiccup must not block the push of the actual code.
        if let Some(wt) = &control.worktree {
            let snap = control.snapshot(id);
            commit_transcript(wt, id, &snap.problem, &snap.participants, &snap.transcript);
        }
        // The worktree is where commits land; the branch ref lives in the repo's
        // shared object store, so pushing from `repo` reaches the same commits.
        push_room_branch(&control.repo, &branch)
    }

    /// Pull a colleague's commits from the remote `room/<id>` branch into this
    /// room's live worktree — the inbound half of cowork. Where `share` hands the
    /// room out for review, `sync` brings reviewed/extended work back so the next
    /// turn builds on top of it. Safe to call between turns; refuses on a dirty
    /// worktree and aborts cleanly on conflict (see `pull_room_branch`).
    pub fn sync(&self, id: &str) -> AppResult<SyncResult> {
        let control = self
            .runs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("roundtable {id}")))?;
        let branch = control.branch.clone().ok_or_else(|| {
            AppError::Other(
                "this is a conversation room (read-only) — only a working room \
                 has a branch to sync"
                    .into(),
            )
        })?;
        let worktree = control.worktree.clone().ok_or_else(|| {
            AppError::Other(
                "this room has no live worktree to sync into (it may have been \
                 closed) — reopen it as a working room first"
                    .into(),
            )
        })?;
        pull_room_branch(&control.repo, &worktree, &branch)
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
    /// Token budget hit — a soft checkpoint (pauses for the human, like the turn
    /// target), not a hard end. The run stays alive and `continue_run` raises the
    /// budget.
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
    // Carry the one-time room notice (e.g. "running read-only") on the first status
    // so it surfaces as the feed banner; later running emits pass None and the
    // frontend keeps the last message.
    emit_status(&app, &id, "running", control.turn_no.load(Ordering::SeqCst), total_tokens, control.notice.clone());

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
        let prompt = build_room_prompt(&control.problem, &participant, &control.participants, &delta, turn, target, control.worktree.is_some());

        // Working room runs the turn in the isolated worktree with edits allowed;
        // a conversation room runs read-only in the project root.
        let cwd = control.workspace.clone();
        let tools = control.tools;
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
                tools,
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
        autosave(&control, &id);
        // Working room: checkpoint whatever this turn edited as one commit on the
        // room branch, so the diff is inspectable per turn and the work survives
        // even if the worktree dir is later cleared. Best-effort — a turn that
        // touched nothing simply leaves no commit, and a git hiccup never kills
        // the conversation.
        if let Some(wt) = &control.worktree {
            commit_worktree(wt, &format!("room {id} · t{turn} {}", participant.name));
        }
        emit_turn(&app, &id, &msg, false, total_tokens, outcome.cost_usd);

        let budget = control.token_budget.load(Ordering::SeqCst);
        if budget > 0 && total_tokens >= budget {
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
            // A checkpoint, not a wall: land in "awaiting" (same as the turn limit)
            // so the human can add a message and/or continue — `continue_run` then
            // grants another budget window. Guard against painting over a continue
            // that already re-acquired the driver in the release window.
            if !control.driving.load(Ordering::SeqCst) {
                let budget = control.token_budget.load(Ordering::SeqCst);
                emit_status(
                    &app,
                    &id,
                    "awaiting",
                    turn,
                    total_tokens,
                    Some(format!(
                        "hit the token budget (~{}k tokens) — add a message or continue",
                        budget / 1000
                    )),
                );
            }
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
    can_edit: bool,
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

    // Two modes: a read-only discussion, or a working room where edits are the
    // deliverable. In the working room the agents share an isolated worktree
    // (changes land on a branch the human reviews before merging), so the prompt
    // tells them to actually implement — otherwise, even with edit permission,
    // they default to merely discussing.
    let mandate = if can_edit {
        "one of several collaborators ({others}) plus a human, working together to solve a real problem **by editing the code**. You're in an isolated worktree: your file changes are committed to a separate branch and reviewed by the human before anything merges — so make concrete edits, don't just describe them."
    } else {
        "one of several collaborators ({others}) plus a human, working together in a shared conversation to solve a real problem. You may READ the open project to ground your reasoning, but you cannot edit files — this is a discussion, not an implementation task."
    }
    .replace("{others}", &others);

    let edit_bullet = if can_edit {
        "\n- Actually make the edits in the files — implement your part directly; the next collaborator builds on your committed changes. Keep each turn's change focused and coherent."
    } else {
        ""
    };

    format!(
        r#"You are **{name}** ({model}), {mandate}
{role}
The problem:
"""
{problem}
"""

This is turn {turn} of {max_turns}.

{convo}

How to contribute:
- Build on what others said. Add what's missing, sharpen what's vague, and say clearly when you disagree and why — but aim to converge on the best answer together, not to win.{edit_bullet}
- Treat the human's messages as high-priority steering.
- Ground claims in the actual code where relevant: read files before asserting how things work.
- Be concise and substantive. One strong contribution per turn beats a wall of text.
- If you believe the room has reached a good answer, say so and summarize it rather than manufacturing more discussion."#,
        name = me.name,
        model = me.model,
        mandate = mandate,
        role = role,
        problem = problem.trim(),
        turn = turn,
        max_turns = max_turns,
        convo = convo,
        edit_bullet = edit_bullet,
    )
}

fn is_safe_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 64
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

// ----- Working-room worktrees (collaborative: one shared, isolated checkout) -----

/// Where a working room's isolated checkout lives. The commits land on the
/// `room/<id>` branch (in the repo's object store), so the work survives even if
/// this temp dir is cleared.
fn room_worktree_path(id: &str) -> PathBuf {
    std::env::temp_dir().join("agent-console-rooms").join(id)
}

/// Create the working room's worktree on a fresh `room/<id>` branch off HEAD.
/// Fails if the repo has no commits (a worktree must branch off something).
fn add_room_worktree(repo: &Path, path: &Path, branch: &str) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let out = proc::command("git")
        .args(["worktree", "add", "-b", branch, &path.to_string_lossy(), "HEAD"])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(AppError::Other(format!(
            "git worktree add (a working room needs a repo with at least one commit): {}",
            msg.trim()
        )));
    }
    Ok(())
}

/// Tear down a working room's checkout. Keeps the `room/<id>` branch (and its
/// commits) — only the temp checkout is removed.
fn remove_room_worktree(repo: &Path, path: &Path) {
    let _ = proc::command("git")
        .args(["worktree", "remove", "--force", &path.to_string_lossy()])
        .current_dir(repo)
        .output();
    let _ = proc::command("git")
        .args(["worktree", "prune"])
        .current_dir(repo)
        .output();
}

/// Checkpoint whatever the latest turn edited as one commit on the room branch.
/// Best-effort and no-op when the turn changed nothing (no empty commits). Uses
/// the user's own git identity (inherited from the repo/global config).
fn commit_worktree(wt: &Path, message: &str) {
    let _ = proc::command("git").args(["add", "-A"]).current_dir(wt).output();
    // `git diff --cached --quiet` exits non-zero exactly when something is staged.
    let staged = proc::command("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(wt)
        .output();
    let has_changes = matches!(staged, Ok(o) if !o.status.success());
    if !has_changes {
        return;
    }
    let _ = proc::command("git")
        .args(["commit", "--no-verify", "-m", message])
        .current_dir(wt)
        .output();
}

/// Render the room's conversation as a self-contained Markdown document so a
/// reviewer reads the full reasoning alongside the diff in the MR/PR. Append-only
/// in shape (turns in order), which keeps re-`share` merges clean.
fn render_transcript_md(
    id: &str,
    problem: &str,
    participants: &[Participant],
    transcript: &[Message],
) -> String {
    let roster = participants
        .iter()
        .map(|p| {
            let engine = match p.engine {
                Engine::Claude => "claude",
                Engine::Codex => "codex",
            };
            let role = if p.role.trim().is_empty() {
                String::new()
            } else {
                format!(" — {}", p.role.trim())
            };
            format!("- **{}** ({engine}/{}){role}", p.name, p.model)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let body = transcript
        .iter()
        .map(|m| {
            let who = match m.engine {
                Some(Engine::Claude) => format!("{} (claude/{})", m.author_name, m.model),
                Some(Engine::Codex) => format!("{} (codex/{})", m.author_name, m.model),
                None => format!("{} (human)", m.author_name),
            };
            format!("### Turn {} — {}\n\n{}", m.turn, who, m.text.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "# Room {id} — conversation\n\n\
         _Auto-generated by Agent Console on `share`. The full reasoning behind \
         this branch, for review alongside the diff._\n\n\
         **Problem**\n\n{}\n\n\
         **Participants**\n\n{roster}\n\n---\n\n{body}\n",
        problem.trim()
    )
}

/// Write the transcript artifact into the worktree and commit just that file onto
/// the room branch. Best-effort and no-op when nothing changed (no empty commits),
/// mirroring `commit_worktree`'s discipline so a no-change re-`share` stays quiet.
fn commit_transcript(
    wt: &Path,
    id: &str,
    problem: &str,
    participants: &[Participant],
    transcript: &[Message],
) {
    let dir = wt.join(".room");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let rel = format!(".room/{id}.md");
    let md = render_transcript_md(id, problem, participants, transcript);
    if fs::write(wt.join(&rel), md).is_err() {
        return;
    }
    let _ = proc::command("git").args(["add", &rel]).current_dir(wt).output();
    let staged = proc::command("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(wt)
        .output();
    let has_changes = matches!(staged, Ok(o) if !o.status.success());
    if !has_changes {
        return;
    }
    let _ = proc::command("git")
        .args(["commit", "--no-verify", "-m", &format!("room {id}: update transcript")])
        .current_dir(wt)
        .output();
}

// ----- Sharing a working room with human collaborators (push + MR/PR link) -----

/// Pick the team's remote: prefer "origin" (the convention), else the first
/// configured. Shared by push (outbound) and sync (inbound).
fn pick_remote(repo: &Path) -> AppResult<String> {
    let listed = proc::command("git")
        .args(["remote"])
        .current_dir(repo)
        .output()?;
    let listed = String::from_utf8_lossy(&listed.stdout);
    listed
        .lines()
        .map(str::trim)
        .find(|r| *r == "origin")
        .or_else(|| listed.lines().map(str::trim).find(|r| !r.is_empty()))
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::Other(
                "no git remote configured — add one (git remote add origin <url>) \
                 so colleagues can fetch this room's branch"
                    .into(),
            )
        })
}

/// Whether a local branch exists in the repo. Lets `share` recover a resumed
/// room's `room/<id>` branch by name when the live handle was dropped on resume.
fn branch_exists(repo: &Path, branch: &str) -> bool {
    proc::command("git")
        .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")])
        .current_dir(repo)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Bring a colleague's commits *into* the live working room: fetch the remote
/// `room/<id>` branch and merge it into the worktree the agents are editing. This
/// is the return path of cowork — `share` pushes the room out, `sync` pulls a
/// colleague's work back in so the next turn builds on top of it.
///
/// Safety: the agent loop auto-commits the worktree every turn, so it must NEVER
/// run on a conflicted tree. On a merge conflict we abort cleanly and report the
/// conflicting files for the human to resolve by hand — we never leave the shared
/// checkout half-merged.
fn pull_room_branch(repo: &Path, worktree: &Path, branch: &str) -> AppResult<SyncResult> {
    let remote = pick_remote(repo)?;

    // The worktree may hold uncommitted edits from a turn that's mid-flight (or
    // that changed nothing and so wasn't committed). Merging onto a dirty tree is
    // unsafe, so refuse rather than risk clobbering in-progress work.
    let dirty = proc::command("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree)
        .output()?;
    if !String::from_utf8_lossy(&dirty.stdout).trim().is_empty() {
        return Err(AppError::Other(
            "the room's worktree has uncommitted changes — let the current turn \
             finish (it auto-commits), then sync again"
                .into(),
        ));
    }

    let fetch = proc::command("git")
        .args(["fetch", &remote, branch])
        .current_dir(worktree)
        .output()?;
    if !fetch.status.success() {
        let err = String::from_utf8_lossy(&fetch.stderr);
        // A branch that was never pushed isn't an error worth alarming about.
        if err.contains("couldn't find remote ref") {
            return Ok(SyncResult {
                branch: branch.to_string(),
                remote,
                merged_commits: 0,
                conflicts: Vec::new(),
                message: format!(
                    "Nothing to sync: {branch} isn't on the remote yet (share it first)."
                ),
            });
        }
        return Err(AppError::Other(format!("git fetch failed: {}", err.trim())));
    }

    // How many commits the colleague has that we don't (purely informational).
    let behind = proc::command("git")
        .args(["rev-list", "--count", "HEAD..FETCH_HEAD"])
        .current_dir(worktree)
        .output()?;
    let behind: usize = String::from_utf8_lossy(&behind.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    if behind == 0 {
        return Ok(SyncResult {
            branch: branch.to_string(),
            remote,
            merged_commits: 0,
            conflicts: Vec::new(),
            message: format!("Already up to date with {remote}/{branch}."),
        });
    }

    let merge = proc::command("git")
        .args([
            "merge",
            "--no-edit",
            "-m",
            &format!("room sync: merge colleague work from {remote}/{branch}"),
            "FETCH_HEAD",
        ])
        .current_dir(worktree)
        .output()?;
    if merge.status.success() {
        return Ok(SyncResult {
            branch: branch.to_string(),
            remote,
            merged_commits: behind,
            conflicts: Vec::new(),
            message: format!(
                "Brought in {behind} commit(s) from {remote}/{branch}. The next turn builds on top."
            ),
        });
    }

    // Conflict (or other merge failure): capture the conflicting paths, then abort
    // so the shared worktree returns to a clean, agent-safe state.
    let conflicts: Vec<String> = proc::command("git")
        .args(["diff", "--name-only", "--diff-filter=U"])
        .current_dir(worktree)
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let _ = proc::command("git")
        .args(["merge", "--abort"])
        .current_dir(worktree)
        .output();

    if conflicts.is_empty() {
        let err = String::from_utf8_lossy(&merge.stderr);
        return Err(AppError::Other(format!("git merge failed: {}", err.trim())));
    }
    let list = conflicts.join(", ");
    Ok(SyncResult {
        branch: branch.to_string(),
        remote,
        merged_commits: 0,
        conflicts,
        message: format!(
            "Colleague work on {branch} conflicts with this room in: {list}. \
             Merge was aborted (worktree left clean). Resolve by hand: \
             `git -C {wt} merge {remote}/{branch}`.",
            wt = worktree.display()
        ),
    })
}

/// Detect the team's remote, push the branch with upstream tracking, and derive
/// a "create MR/PR" URL from the remote host. Best-effort URL — the push is what
/// matters; an unrecognized host just yields `pr_url: None`.
fn push_room_branch(repo: &Path, branch: &str) -> AppResult<ShareResult> {
    let remote = pick_remote(repo)?;

    let push = proc::command("git")
        .args(["push", "-u", &remote, branch])
        .current_dir(repo)
        .output()?;
    if !push.status.success() {
        let err = String::from_utf8_lossy(&push.stderr);
        let err = err.trim();
        // The common round-trip snag: a colleague pushed to `room/<id>` after our
        // last sync, so this push is non-fast-forward. Point the human at Sync
        // (which pulls their commits in cleanly) instead of a raw git error.
        if err.contains("non-fast-forward") || err.contains("fetch first") || err.contains("rejected") {
            return Err(AppError::Other(format!(
                "push rejected — a colleague has pushed to {branch} since your last \
                 sync. Click Sync to bring their work in, then Share again. \
                 (git said: {err})"
            )));
        }
        return Err(AppError::Other(format!("git push failed: {err}")));
    }

    let url = proc::command("git")
        .args(["remote", "get-url", &remote])
        .current_dir(repo)
        .output()?;
    let url = String::from_utf8_lossy(&url.stdout).trim().to_string();
    let pr_url = pr_url_for(&url, branch);

    let message = match &pr_url {
        Some(u) => format!("Pushed {branch} → {remote}. Open the MR/PR: {u}"),
        None => format!(
            "Pushed {branch} → {remote}. Open an MR/PR from it in your git host."
        ),
    };
    Ok(ShareResult {
        branch: branch.to_string(),
        remote,
        pr_url,
        message,
    })
}

/// Turn a remote URL into a "create MR/PR for this branch" web URL for the hosts
/// we recognize (GitHub, GitLab). `None` for anything else.
fn pr_url_for(remote_url: &str, branch: &str) -> Option<String> {
    let (host, path) = parse_remote(remote_url)?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    if host.contains("github") {
        Some(format!("https://{host}/{path}/compare/{branch}?expand=1"))
    } else if host.contains("gitlab") {
        Some(format!(
            "https://{host}/{path}/-/merge_requests/new?merge_request%5Bsource_branch%5D={branch}"
        ))
    } else {
        None
    }
}

/// Split a git remote URL into (host, "owner/repo"). Supports scp-like SSH
/// (`git@host:owner/repo.git`), `ssh://`, and `http(s)://`, stripping any
/// `user@` and the trailing `.git` so it round-trips into a web URL.
fn parse_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some((host.to_string(), path.to_string()));
    }
    for scheme in ["https://", "http://", "ssh://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            // Drop a leading user@ (ssh URLs) before the host.
            let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
            let (host, path) = rest.split_once('/')?;
            return Some((host.to_string(), path.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn participant(id: &str) -> Participant {
        Participant {
            id: id.into(),
            name: format!("P-{id}"),
            engine: Engine::default(),
            model: "opus".into(),
            role: String::new(),
        }
    }

    fn message(author: &str, turn: u32) -> Message {
        Message {
            author_id: author.into(),
            author_name: author.into(),
            engine: Some(Engine::default()),
            model: "opus".into(),
            text: format!("msg from {author} on turn {turn}"),
            turn,
        }
    }

    #[test]
    fn pr_url_for_github_and_gitlab_ssh_and_https() {
        let b = "room/r-123";
        // GitHub, both URL forms.
        assert_eq!(
            pr_url_for("git@github.com:acme/widgets.git", b).as_deref(),
            Some("https://github.com/acme/widgets/compare/room/r-123?expand=1")
        );
        assert_eq!(
            pr_url_for("https://github.com/acme/widgets", b).as_deref(),
            Some("https://github.com/acme/widgets/compare/room/r-123?expand=1")
        );
        // GitLab (incl. self-hosted host containing "gitlab").
        assert_eq!(
            pr_url_for("git@gitlab.example.com:team/app.git", b).as_deref(),
            Some("https://gitlab.example.com/team/app/-/merge_requests/new?merge_request%5Bsource_branch%5D=room/r-123")
        );
        // Unrecognized host → no link, but the branch is still pushed.
        assert_eq!(pr_url_for("git@bitbucket.org:x/y.git", b), None);
    }

    #[test]
    fn transcript_md_carries_problem_roster_and_turns() {
        let participants = vec![participant("p1"), participant("p2")];
        // A real human message has engine None — that's what renders as "(human)".
        let human = Message {
            author_id: "human".into(),
            author_name: "Carlos".into(),
            engine: None,
            model: String::new(),
            text: "steer left".into(),
            turn: 1,
        };
        let transcript = vec![message("p1", 1), human];
        let md = render_transcript_md("r-xyz", "Ship cowork", &participants, &transcript);
        // Header, problem, and a participant roster line are present.
        assert!(md.contains("# Room r-xyz — conversation"));
        assert!(md.contains("**Problem**"));
        assert!(md.contains("Ship cowork"));
        assert!(md.contains("**P-p1** (claude/opus)"));
        // Every transcript message becomes a turn section; the human is labeled.
        assert!(md.contains("### Turn 1 — P-p1 (claude/opus)"));
        assert!(md.contains("### Turn 1 — Carlos (human)"));
        assert!(md.contains("steer left"));
    }

    fn room(id: &str, problem: &str, updated_at_ms: u64) -> PersistedRoom {
        PersistedRoom {
            version: ROOM_SCHEMA_VERSION,
            id: id.into(),
            problem: problem.into(),
            participants: vec![participant("p1"), participant("p2")],
            transcript: vec![message("p1", 1), message("human", 1)],
            resume: HashMap::from([("p1".to_string(), "resume-token-p1".to_string())]),
            last_seen: HashMap::from([("p1".to_string(), 2usize)]),
            total_tokens: 4242,
            updated_at_ms,
        }
    }

    /// One test fn on purpose: it mutates the process-global `XDG_DATA_HOME`, so
    /// it must not race a sibling test. Exercises the real load/save code in an
    /// isolated data dir (dirs::data_local_dir respects XDG_DATA_HOME on Linux)
    /// so the user's real rooms.json is never touched.
    #[test]
    fn rooms_persistence_is_crash_safe() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("agent-console-rooms-test-{nanos}"));
        std::env::set_var("XDG_DATA_HOME", &base);

        let store = RoomsStore::new();
        let p1 = Path::new("/proj/one");

        // Empty state: no file yet.
        assert!(store.summaries("/proj/one").unwrap().is_empty());

        // Round trip: save A, read the full room back with every field intact.
        store.save_room(&room("a", "problem A", 100), p1).unwrap();
        let full = store.get("/proj/one", "a").unwrap().expect("room a exists");
        assert_eq!(full.id, "a");
        assert_eq!(full.problem, "problem A");
        assert_eq!(full.version, ROOM_SCHEMA_VERSION);
        assert_eq!(full.transcript.len(), 2);
        assert_eq!(full.transcript[0].text, "msg from p1 on turn 1");
        assert_eq!(full.resume.get("p1").map(String::as_str), Some("resume-token-p1"));
        assert_eq!(full.last_seen.get("p1"), Some(&2usize));
        assert_eq!(full.total_tokens, 4242);
        // Summary derives its fields from the room.
        let sum = store.summaries("/proj/one").unwrap();
        assert_eq!(sum.len(), 1);
        assert_eq!(sum[0].id, "a");
        assert_eq!(sum[0].problem, "problem A");
        assert_eq!(sum[0].message_count, 2);
        assert_eq!(sum[0].last_turn, 1);
        assert_eq!(sum[0].participant_names, vec!["P-p1", "P-p2"]);

        // Add B (distinct id), then upsert A in place — count stays 2, A updates.
        store.save_room(&room("b", "problem B", 200), p1).unwrap();
        store.save_room(&room("a", "problem A v2", 300), p1).unwrap();
        let sum = store.summaries("/proj/one").unwrap();
        assert_eq!(sum.len(), 2, "upsert must not duplicate an existing id");
        // Sorted most-recently-updated first: A (300) before B (200).
        assert_eq!(sum[0].id, "a");
        assert_eq!(sum[0].problem, "problem A v2");
        assert_eq!(sum[1].id, "b");
        // A missing id resolves to None.
        assert!(store.get("/proj/one", "nope").unwrap().is_none());

        // Cross-project isolation: a different project is untouched.
        let p2 = Path::new("/proj/two");
        assert!(store.summaries("/proj/two").unwrap().is_empty());

        // Retention: 51 rooms in p2 prune to the most-recent 50; the oldest drops.
        for i in 0..=MAX_ROOMS_PER_PROJECT {
            store
                .save_room(&room(&format!("r{i}"), "x", i as u64), p2)
                .unwrap();
        }
        let kept = store.summaries("/proj/two").unwrap();
        assert_eq!(kept.len(), MAX_ROOMS_PER_PROJECT);
        assert!(!kept.iter().any(|r| r.id == "r0"), "oldest room must be pruned");

        // Delete: remove B from p1, then A — emptying p1 drops the project key.
        store.delete_room("/proj/one", "b").unwrap();
        assert_eq!(store.summaries("/proj/one").unwrap().len(), 1);
        store.delete_room("/proj/one", "a").unwrap();
        assert!(store.summaries("/proj/one").unwrap().is_empty());
        store.delete_room("/proj/one", "a").unwrap(); // idempotent

        // Crash safety: corrupting the live file falls back to the `.bak` (which
        // holds the prior good full state — p2 still has its 50 rooms).
        let main = base.join("agent-console").join("rooms.json");
        fs::write(&main, b"{ this is not valid json ]").unwrap();
        let recovered = store.summaries("/proj/two").unwrap();
        assert_eq!(recovered.len(), MAX_ROOMS_PER_PROJECT, "must recover p2 from .bak");

        let _ = fs::remove_dir_all(&base);
    }

    /// Fase B: rebuilding a live run from a persisted room. Pure in-memory (no
    /// disk, no env), so it's independent of the persistence test above.
    #[test]
    fn restore_rebuilds_a_live_run() {
        let svc = RoundtableService::new();
        let repo = std::env::temp_dir(); // a real, existing directory

        // Happy path: keeps the room's own id so it continues the same history.
        let r = room("r-keep-id", "continue me", 1);
        assert_eq!(svc.restore(repo.clone(), r.clone()).unwrap(), "r-keep-id");
        // Idempotent: re-restoring a live id returns it, leaving the run intact.
        assert_eq!(svc.restore(repo.clone(), r).unwrap(), "r-keep-id");

        // Rejects a repo that isn't a directory.
        assert!(svc
            .restore(repo.join("nope-xyz-123"), room("r-a", "p", 1))
            .is_err());

        // Rejects fewer than two participants.
        let mut solo = room("r-b", "p", 1);
        solo.participants.truncate(1);
        assert!(svc.restore(repo.clone(), solo).is_err());

        // Rejects a shell-unsafe model value (defense in depth before a PTY).
        let mut bad = room("r-c", "p", 1);
        bad.participants[0].model = "opus; rm -rf /".into();
        assert!(svc.restore(repo, bad).is_err());
    }

    /// W1: the working-room worktree mechanic against a real throwaway git repo —
    /// create off HEAD, checkpoint an edit per turn, no-op turns add nothing, and
    /// teardown keeps the branch. Hermetic (temp dir + temp repo, no env, no net).
    #[test]
    fn working_room_worktree_lifecycle() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let repo = std::env::temp_dir().join(format!("ac-wt-test-{nanos}"));
        fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str], cwd: &Path| {
            proc::command("git").args(args).current_dir(cwd).output().unwrap()
        };
        // Minimal repo with one commit — a worktree must branch off something.
        git(&["init", "-q"], &repo);
        git(&["config", "user.email", "t@t"], &repo);
        git(&["config", "user.name", "T"], &repo);
        fs::write(repo.join("seed.txt"), "seed").unwrap();
        git(&["add", "-A"], &repo);
        git(&["commit", "-qm", "seed"], &repo);

        let count = |branch: &str| -> usize {
            let out = git(&["rev-list", "--count", branch], &repo);
            String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
        };

        // Create the working-room worktree on a fresh branch off HEAD.
        let wt = room_worktree_path(&format!("wt-{nanos}"));
        add_room_worktree(&repo, &wt, "room/wt-test").unwrap();
        assert!(wt.join("seed.txt").exists(), "worktree checks out HEAD");
        assert_eq!(count("room/wt-test"), 1);

        // An edited turn is checkpointed as exactly one commit.
        fs::write(wt.join("agent.txt"), "edit").unwrap();
        commit_worktree(&wt, "t1");
        assert_eq!(count("room/wt-test"), 2, "an edited turn adds one commit");

        // A turn that changed nothing leaves no empty commit.
        commit_worktree(&wt, "t2-noop");
        assert_eq!(count("room/wt-test"), 2, "a no-op turn adds no commit");

        // Teardown removes the checkout but keeps the branch and its commits.
        remove_room_worktree(&repo, &wt);
        assert!(!wt.exists(), "checkout dir removed on teardown");
        assert_eq!(count("room/wt-test"), 2, "branch + commits survive teardown");

        let _ = fs::remove_dir_all(&repo);
    }

    /// `branch_exists` is what lets a *resumed* room (which lost its live branch
    /// handle) still Share by recovering `room/<id>` by name. Hermetic temp repo.
    #[test]
    fn branch_exists_detects_room_branch() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let repo = std::env::temp_dir().join(format!("ac-be-test-{nanos}"));
        fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            proc::command("git").args(args).current_dir(&repo).output().unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "T"]);
        fs::write(repo.join("seed.txt"), "seed").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "seed"]);

        assert!(!branch_exists(&repo, "room/r-missing"), "absent branch → false");
        git(&["branch", "room/r-here"]);
        assert!(branch_exists(&repo, "room/r-here"), "created branch → true");

        let _ = fs::remove_dir_all(&repo);
    }

    /// The inbound half of cowork: a colleague's commits on the remote `room/…`
    /// branch are fetched and merged into the live worktree, and a conflicting
    /// change is reported and aborted cleanly (never left half-merged, because the
    /// agent loop auto-commits the worktree). Hermetic: bare remote + temp clones.
    #[test]
    fn room_sync_pulls_colleague_commits_and_aborts_on_conflict() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let git = |args: &[&str], cwd: &Path| {
            proc::command("git").args(args).current_dir(cwd).output().unwrap()
        };
        let tmp = std::env::temp_dir();

        // A bare repo standing in for the team's shared remote.
        let remote = tmp.join(format!("ac-sync-remote-{nanos}"));
        fs::create_dir_all(&remote).unwrap();
        git(&["init", "-q", "--bare"], &remote);
        let remote_url = remote.to_string_lossy().to_string();

        // The room's repo, with one seed commit and `origin` pointing at the remote.
        let repo = tmp.join(format!("ac-sync-repo-{nanos}"));
        fs::create_dir_all(&repo).unwrap();
        git(&["init", "-q"], &repo);
        git(&["config", "user.email", "t@t"], &repo);
        git(&["config", "user.name", "T"], &repo);
        fs::write(repo.join("seed.txt"), "seed\n").unwrap();
        git(&["add", "-A"], &repo);
        git(&["commit", "-qm", "seed"], &repo);
        git(&["remote", "add", "origin", &remote_url], &repo);

        // The room's live worktree on a fresh branch, pushed to the remote.
        let branch = "room/sync-test";
        let wt = room_worktree_path(&format!("sync-{nanos}"));
        add_room_worktree(&repo, &wt, branch).unwrap();
        git(&["push", "-q", "-u", "origin", branch], &repo);

        // A colleague clones, adds a non-conflicting file, and pushes.
        let colab = tmp.join(format!("ac-sync-colab-{nanos}"));
        git(&["clone", "-q", &remote_url, &colab.to_string_lossy()], &tmp);
        git(&["config", "user.email", "c@c"], &colab);
        git(&["config", "user.name", "C"], &colab);
        git(&["checkout", "-q", branch], &colab);
        fs::write(colab.join("colab.txt"), "from colleague\n").unwrap();
        git(&["add", "-A"], &colab);
        git(&["commit", "-qm", "colleague feature"], &colab);
        git(&["push", "-q", "origin", branch], &colab);

        // Sync brings the colleague's commit into the live worktree.
        let res = pull_room_branch(&repo, &wt, branch).unwrap();
        assert_eq!(res.merged_commits, 1, "one colleague commit merged in");
        assert!(res.conflicts.is_empty(), "clean merge has no conflicts");
        assert!(wt.join("colab.txt").exists(), "colleague's file landed in the worktree");

        // A second sync with nothing new is a clean no-op.
        let again = pull_room_branch(&repo, &wt, branch).unwrap();
        assert_eq!(again.merged_commits, 0, "already up to date");

        // Now both sides edit the same file → conflict. Room turn edits seed.txt…
        fs::write(wt.join("seed.txt"), "room version\n").unwrap();
        commit_worktree(&wt, "room turn edits seed");
        // …colleague edits the same line and pushes.
        fs::write(colab.join("seed.txt"), "colleague version\n").unwrap();
        git(&["add", "-A"], &colab);
        git(&["commit", "-qm", "colleague edits seed"], &colab);
        git(&["push", "-q", "origin", branch], &colab);

        let conflict = pull_room_branch(&repo, &wt, branch).unwrap();
        assert_eq!(conflict.merged_commits, 0, "conflicting merge brings nothing in");
        assert_eq!(conflict.conflicts, vec!["seed.txt".to_string()], "reports the conflicting file");
        // The merge was aborted: the worktree is clean and keeps the room's version.
        let status = git(&["status", "--porcelain"], &wt);
        assert!(
            String::from_utf8_lossy(&status.stdout).trim().is_empty(),
            "worktree is clean after abort (safe for the next auto-committing turn)"
        );
        assert_eq!(fs::read_to_string(wt.join("seed.txt")).unwrap(), "room version\n");

        remove_room_worktree(&repo, &wt);
        let _ = fs::remove_dir_all(&repo);
        let _ = fs::remove_dir_all(&remote);
        let _ = fs::remove_dir_all(&colab);
    }

    #[test]
    fn worktree_creation_fails_without_a_git_repo() {
        // A plain folder (no git, or a repo with no commits) can't host a worktree.
        // `start` catches exactly this Err and degrades the room to read-only
        // instead of failing — so opening a non-git workspace (Jira/GitLab/MCP
        // only) still works, just without agent editing.
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let plain = std::env::temp_dir().join(format!("ac-norepo-test-{nanos}"));
        fs::create_dir_all(&plain).unwrap();

        let wt = room_worktree_path(&format!("norepo-{nanos}"));
        let res = add_room_worktree(&plain, &wt, "room/norepo-test");
        assert!(res.is_err(), "a non-git folder cannot host a working-room worktree");
        assert!(!wt.exists(), "no stray checkout left behind on failure");

        let _ = fs::remove_dir_all(&plain);
    }
}
