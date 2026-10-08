//! Connector: the agent-facing side of a room. Agents reach it through the
//! `agent_console` MCP server the bridge exposes to each headless turn, and use
//! it to delegate work to a peer, ask the user a question with selectable
//! options, record a review verdict and poll their own tasks. The room's
//! worker drives the queue (run the recipient's turn, then wake the sender with
//! the result), so an agent can hand off and END its turn instead of waiting.
//!
//! Port of `ai_connector` by Marcos Macías (mmaciass/ai-connector, used with
//! his permission): `persistence/project/tasks.py`, `domain/questions.py`,
//! `persistence/project/reviews.py` and the dispatcher in `mcp/server.py`.
//! SQLite there becomes the crate's crash-safe JSON pattern here, and the
//! participant/job model maps onto room participants / room id. Rules kept
//! verbatim where they are user-visible contracts: idempotent `request_key`,
//! the ten-pending-tasks limit, the six-option cap and every validation
//! message an agent can read back.
//!
//! The GUI process is the only writer: the bridge relays each tool call over
//! the loopback `POST /mcp` route, so no two processes ever touch
//! `connector.json` at once.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

/// Caps mirrored from ai-connector's contract.
const MAX_INSTRUCTIONS: usize = 16_000;
const MAX_REQUEST_KEY: usize = 160;
const MAX_PENDING_TASKS: usize = 10;
const MAX_OPTIONS: usize = 6;
const MAX_OPTION_ID: usize = 80;
const MAX_OPTION_LABEL: usize = 120;
const MAX_OPTION_DESCRIPTION: usize = 300;
/// Retention per project before the oldest finished records are dropped.
const KEEP_TASKS: usize = 300;
const KEEP_QUESTIONS: usize = 200;
const KEEP_REVIEWS: usize = 200;
const KEEP_PENDING_JOBS: usize = 100;
/// Tasks one room may leave waiting for approval (ai-connector's limit).
const MAX_PENDING_JOBS: usize = 10;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Domain
// ---------------------------------------------------------------------------

/// What a participant is allowed to be inside one job. Mirrors
/// `domain/policies.py::ROLES`.
#[allow(dead_code)] // consumed by the room wiring (phase 3)
pub const ROLES: &[&str] = &[
    "organizer",
    "implementer",
    "reviewer",
    "planner",
    "consultant",
    "assistant",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    /// Opaque participant id — the room participant id ("p1", "p2", …).
    pub id: String,
    /// One or more of [`ROLES`]; the first is the primary role.
    pub roles: Vec<String>,
}

/// The authorized team of one job (= one room). Registered by the room before
/// its first turn; every tool call is checked against it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub job_id: String,
    pub members: Vec<TeamMember>,
    /// Revision label a review is recorded against (bumped by the room after
    /// each implementation turn). ai-connector derives it from the last
    /// implementation turn id; here the room owns the counter.
    #[serde(default = "initial_revision")]
    pub revision: String,
}

fn initial_revision() -> String {
    "initial".into()
}

impl Team {
    pub fn member(&self, id: &str) -> Option<&TeamMember> {
        self.members.iter().find(|m| m.id == id)
    }
    pub fn has_role(&self, id: &str, role: &str) -> bool {
        self.member(id)
            .is_some_and(|m| m.roles.iter().any(|r| r == role))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Task,
    Consult,
    Discussion,
    Review,
    Correction,
}

/// Lifecycle of a delegated task. `Queued` → the worker runs the recipient's
/// turn (`Executing`) → the result waits for the sender to be idle (`Ready`) →
/// the worker wakes the sender with it (`Delivering`) → `Delivered`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStage {
    Queued,
    Executing,
    Ready,
    Delivering,
    Delivered,
    DeliveryFailed,
    Cancelled,
}

impl TaskStage {
    /// A pending task counts against the per-job limit.
    pub fn is_pending(self) -> bool {
        !matches!(
            self,
            Self::Delivered | Self::DeliveryFailed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub job_id: String,
    pub sender: String,
    pub recipient: String,
    pub request_key: String,
    pub instructions: String,
    pub kind: TaskKind,
    pub stage: TaskStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// What the sender said when the result was handed back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

/// Fields the worker may change on a task. Mirrors `update_task`'s allow-list.
#[allow(dead_code)] // constructed by the room worker (phase 3)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPatch {
    pub stage: Option<TaskStage>,
    pub outcome: Option<Outcome>,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionStatus {
    Waiting,
    Answered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    /// The text the asker receives: the chosen option's label, or free text.
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
    pub answered_ms: u64,
    /// Set once the worker fed the answer into the asker's next turn.
    #[serde(default)]
    pub delivered: bool,
}

/// An `ask_user` question. The asker ends its turn; the room shows the
/// question, the user answers (option or free text), the worker resumes the
/// asker with the answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    pub job_id: String,
    pub sender: String,
    pub body: String,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    pub status: QuestionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
    pub created_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Approved,
    Changes,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub id: String,
    pub job_id: String,
    pub participant: String,
    pub revision: String,
    pub verdict: Verdict,
    pub body: String,
    pub created_ms: u64,
}

/// `create_task`: work an agent split off or postponed, waiting for the human
/// to approve it as a new job (room) with the same team. Port of
/// ai-connector's pending-approval jobs, minus the kanban column.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingStatus {
    PendingApproval,
    Approved,
    Discarded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingJob {
    pub id: String,
    /// The room whose agent recorded it.
    pub source_job_id: String,
    pub creator: String,
    pub instructions: String,
    pub request_key: String,
    pub status: PendingStatus,
    /// The room the approval started, once approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_job_id: Option<String>,
    pub created_ms: u64,
}

/// Normalize `ask_user.options` to `[{id,label,description}]`. Port of
/// `normalize_question_options`, messages included — agents read them back.
pub fn normalize_question_options(options: Option<&Value>) -> Result<Vec<QuestionOption>, String> {
    let Some(options) = options else {
        return Ok(Vec::new());
    };
    if options.is_null() {
        return Ok(Vec::new());
    }
    let Some(items) = options.as_array() else {
        return Err("Invalid question options".into());
    };
    if items.len() > MAX_OPTIONS {
        return Err("A question supports up to 6 options".into());
    }
    let mut normalized = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let (label, given_id, description): (Option<&Value>, Option<&Value>, Option<&Value>) =
            match item {
                Value::String(_) => (Some(item), None, None),
                Value::Object(map) => {
                    if map
                        .keys()
                        .any(|k| !matches!(k.as_str(), "id" | "label" | "description"))
                    {
                        return Err("Invalid question option".into());
                    }
                    (map.get("label"), map.get("id"), map.get("description"))
                }
                _ => return Err("Invalid question option".into()),
            };
        let label = match label.and_then(Value::as_str) {
            Some(l) if (1..=MAX_OPTION_LABEL).contains(&l.trim().chars().count()) => {
                l.trim().to_string()
            }
            _ => return Err("Invalid option label".into()),
        };
        let id = match given_id {
            None | Some(Value::Null) => format!("opt-{}", index + 1),
            Some(Value::String(s)) => {
                let s = s.trim();
                let ok = (1..=MAX_OPTION_ID).contains(&s.chars().count())
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
                if !ok {
                    return Err("Invalid option identifier".into());
                }
                s.to_string()
            }
            Some(_) => return Err("Invalid option identifier".into()),
        };
        let description = match description {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(d)) if d.chars().count() <= MAX_OPTION_DESCRIPTION => {
                d.trim().to_string()
            }
            Some(_) => return Err("Invalid option description".into()),
        };
        normalized.push(QuestionOption {
            id,
            label,
            description,
        });
    }
    let mut ids: Vec<&str> = normalized.iter().map(|o| o.id.as_str()).collect();
    let mut labels: Vec<&str> = normalized.iter().map(|o| o.label.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    labels.sort_unstable();
    labels.dedup();
    if ids.len() != normalized.len() || labels.len() != normalized.len() {
        return Err("Duplicate question options".into());
    }
    Ok(normalized)
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConnector {
    #[serde(default)]
    pub teams: HashMap<String, Team>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub questions: Vec<Question>,
    #[serde(default)]
    pub reviews: Vec<Review>,
    #[serde(default)]
    pub pending_jobs: Vec<PendingJob>,
}

impl ProjectConnector {
    fn is_empty(&self) -> bool {
        self.teams.is_empty()
            && self.tasks.is_empty()
            && self.questions.is_empty()
            && self.reviews.is_empty()
            && self.pending_jobs.is_empty()
    }

    /// Drop the oldest FINISHED records past the caps. Pending work is never
    /// dropped by retention — only the worker moves it on.
    fn trim(&mut self) {
        while self.tasks.len() > KEEP_TASKS {
            match self.tasks.iter().position(|t| !t.stage.is_pending()) {
                Some(i) => {
                    tracing::debug!("connector: retention dropped task {}", self.tasks[i].id);
                    self.tasks.remove(i);
                }
                None => break,
            }
        }
        while self.questions.len() > KEEP_QUESTIONS {
            match self
                .questions
                .iter()
                .position(|q| q.status == QuestionStatus::Answered)
            {
                Some(i) => {
                    self.questions.remove(i);
                }
                None => break,
            }
        }
        if self.reviews.len() > KEEP_REVIEWS {
            let drop = self.reviews.len() - KEEP_REVIEWS;
            self.reviews.drain(..drop);
        }
        while self.pending_jobs.len() > KEEP_PENDING_JOBS {
            match self
                .pending_jobs
                .iter()
                .position(|j| j.status != PendingStatus::PendingApproval)
            {
                Some(i) => {
                    self.pending_jobs.remove(i);
                }
                None => break,
            }
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConnectorFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    by_project: HashMap<String, ProjectConnector>,
}

pub struct ConnectorService {
    lock: Mutex<()>,
}

impl Default for ConnectorService {
    fn default() -> Self {
        Self::new()
    }
}

// The team/worker API is driven by the room (phase 3); until then only the
// MCP dispatcher below and the tests call it.
#[allow(dead_code)]
impl ConnectorService {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
    fn path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("connector.json"))
    }
    fn bak_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("connector.json.bak"))
    }
    fn tmp_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("connector.json.tmp"))
    }

    fn load_file() -> AppResult<ConnectorFile> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(ConnectorFile::default());
        }
        let txt = fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("read connector.json: {e}")))?;
        if txt.trim().is_empty() {
            return Ok(ConnectorFile::default());
        }
        match serde_json::from_str::<ConnectorFile>(&txt) {
            Ok(file) => Ok(file),
            Err(e) => {
                if let Ok(bak) = Self::bak_path() {
                    if let Ok(btxt) = fs::read_to_string(&bak) {
                        if let Ok(file) = serde_json::from_str::<ConnectorFile>(&btxt) {
                            return Ok(file);
                        }
                    }
                }
                Err(AppError::Other(format!("parse connector.json: {e}")))
            }
        }
    }

    fn write_file(file: &ConnectorFile) -> AppResult<()> {
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
        let mut last = None;
        for attempt in 0..3 {
            match fs::rename(&tmp, &path) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last = Some(e);
                    if attempt < 2 {
                        std::thread::sleep(std::time::Duration::from_millis(80));
                    }
                }
            }
        }
        Err(last.expect("loop ran").into())
    }

    /// Read-modify-write one project's connector state under the lock. The
    /// closure returns the value to hand back; `Err(String)` is a tool-level
    /// refusal (persisted nothing), `AppError` an I/O failure.
    fn mutate<T>(
        &self,
        project: &str,
        f: impl FnOnce(&mut ProjectConnector) -> Result<T, String>,
    ) -> Result<T, CallError> {
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        let entry = file.by_project.entry(project.to_string()).or_default();
        let before = serde_json::to_string(entry).unwrap_or_default();
        let out = f(entry).map_err(CallError::Tool)?;
        entry.trim();
        let after = serde_json::to_string(entry).unwrap_or_default();
        if entry.is_empty() {
            file.by_project.remove(project);
        }
        if before != after {
            file.version = 1;
            Self::write_file(&file)?;
        }
        Ok(out)
    }

    fn read<T>(&self, project: &str, f: impl FnOnce(&ProjectConnector) -> T) -> AppResult<T> {
        let _g = self.lock.lock();
        let file = Self::load_file()?;
        Ok(f(file
            .by_project
            .get(project)
            .unwrap_or(&ProjectConnector::default())))
    }

    // ----- team -----------------------------------------------------------

    /// Register (or replace) the authorized team of a job. Roles outside
    /// [`ROLES`] and duplicate ids are refused.
    pub fn register_team(&self, project: &str, team: Team) -> AppResult<()> {
        let mut seen = std::collections::HashSet::new();
        for m in &team.members {
            if m.id.trim().is_empty() || !seen.insert(m.id.as_str()) {
                return Err(AppError::InvalidArgument(format!(
                    "duplicate or empty participant id: {}",
                    m.id
                )));
            }
            if m.roles.is_empty() || m.roles.iter().any(|r| !ROLES.contains(&r.as_str())) {
                return Err(AppError::InvalidArgument(format!(
                    "invalid roles for {}: {:?}",
                    m.id, m.roles
                )));
            }
        }
        let job = team.job_id.clone();
        self.mutate(project, |p| {
            // Re-registering (every driver start) refreshes the roster but must
            // not reset the revision reviews are recorded against — that made a
            // continued job re-review work it had already approved.
            let mut team = team;
            if let Some(existing) = p.teams.get(&job) {
                team.revision = existing.revision.clone();
            }
            p.teams.insert(job, team);
            Ok(())
        })
        .map_err(CallError::into_app)
    }

    pub fn team(&self, project: &str, job_id: &str) -> AppResult<Option<Team>> {
        self.read(project, |p| p.teams.get(job_id).cloned())
    }

    /// Bump the revision reviews are recorded against (after an implementation turn).
    pub fn set_revision(&self, project: &str, job_id: &str, revision: &str) -> AppResult<()> {
        let rev = revision.to_string();
        self.mutate(project, |p| {
            let team = p.teams.get_mut(job_id).ok_or("Unknown job")?;
            team.revision = rev;
            Ok(())
        })
        .map_err(CallError::into_app)
    }

    /// Forget a finished job: its team and every record. Rooms call this when
    /// the user deletes the room.
    pub fn forget_job(&self, project: &str, job_id: &str) -> AppResult<()> {
        self.mutate(project, |p| {
            p.teams.remove(job_id);
            p.tasks.retain(|t| t.job_id != job_id);
            p.questions.retain(|q| q.job_id != job_id);
            p.reviews.retain(|r| r.job_id != job_id);
            p.pending_jobs.retain(|j| j.source_job_id != job_id);
            Ok(())
        })
        .map_err(CallError::into_app)
    }

    // ----- pending jobs (create_task) ------------------------------------

    /// `create_task`: record work to split off or postpone. Idempotent on
    /// `(source job, request_key)`; at most ten waiting per room.
    pub fn create_task(
        &self,
        project: &str,
        job_id: &str,
        caller: &str,
        instructions: &str,
        request_key: &str,
    ) -> Result<PendingJob, CallError> {
        if !(1..=MAX_INSTRUCTIONS).contains(&instructions.trim().chars().count()) {
            return Err(CallError::Tool("Invalid instructions".into()));
        }
        if !(1..=MAX_REQUEST_KEY).contains(&request_key.chars().count()) {
            return Err(CallError::Tool("Invalid request_key".into()));
        }
        let (caller, instructions, request_key) = (
            caller.to_string(),
            instructions.to_string(),
            request_key.to_string(),
        );
        self.mutate(project, |p| {
            let team = p
                .teams
                .get(job_id)
                .ok_or("Create tasks only from a job in progress")?;
            if team.member(&caller).is_none() {
                return Err("The AI does not participate in this job".into());
            }
            if let Some(old) = p
                .pending_jobs
                .iter()
                .find(|j| j.source_job_id == job_id && j.request_key == request_key)
            {
                if old.instructions != instructions {
                    return Err(
                        "The same request_key already belongs to another instruction".into(),
                    );
                }
                return Ok(old.clone());
            }
            let waiting = p
                .pending_jobs
                .iter()
                .filter(|j| j.source_job_id == job_id && j.status == PendingStatus::PendingApproval)
                .count();
            if waiting >= MAX_PENDING_JOBS {
                return Err("Each job may have at most 10 tasks waiting for approval".into());
            }
            let job = PendingJob {
                id: uuid::Uuid::new_v4().to_string(),
                source_job_id: job_id.to_string(),
                creator: caller,
                instructions,
                request_key,
                status: PendingStatus::PendingApproval,
                approved_job_id: None,
                created_ms: now_ms(),
            };
            p.pending_jobs.push(job.clone());
            Ok(job)
        })
    }

    pub fn pending_jobs(&self, project: &str, job_id: &str) -> AppResult<Vec<PendingJob>> {
        self.read(project, |p| {
            p.pending_jobs
                .iter()
                .filter(|j| j.source_job_id == job_id)
                .cloned()
                .collect()
        })
    }

    /// The human approved (with the room it started) or discarded a pending task.
    pub fn resolve_pending(
        &self,
        project: &str,
        pending_id: &str,
        approved_job_id: Option<&str>,
    ) -> AppResult<PendingJob> {
        let approved = approved_job_id.map(str::to_string);
        self.mutate(project, |p| {
            let job = p
                .pending_jobs
                .iter_mut()
                .find(|j| j.id == pending_id)
                .ok_or("Unknown pending task")?;
            if job.status != PendingStatus::PendingApproval {
                return Err("This task was already resolved".into());
            }
            job.status = if approved.is_some() {
                PendingStatus::Approved
            } else {
                PendingStatus::Discarded
            };
            job.approved_job_id = approved;
            Ok(job.clone())
        })
        .map_err(CallError::into_app)
    }

    // ----- tasks ----------------------------------------------------------

    /// `delegate_task` / `send_message`: queue work for a peer. Idempotent on
    /// `(job, sender, request_key)`: the same request returns the saved task,
    /// a different one under the same key is refused.
    #[allow(clippy::too_many_arguments)] // one call site per tool; a struct would only rename the fields
    pub fn delegate(
        &self,
        project: &str,
        job_id: &str,
        sender: &str,
        recipient: &str,
        instructions: &str,
        request_key: &str,
        kind: TaskKind,
    ) -> Result<Task, CallError> {
        let trimmed_len = instructions.trim().chars().count();
        if !(1..=MAX_INSTRUCTIONS).contains(&trimmed_len) {
            return Err(CallError::Tool("Invalid instructions".into()));
        }
        if !(1..=MAX_REQUEST_KEY).contains(&request_key.chars().count()) {
            return Err(CallError::Tool("Invalid request_key".into()));
        }
        if sender == recipient {
            return Err(CallError::Tool(
                "Invalid delegation: another participant in the same project is required".into(),
            ));
        }
        let (sender, recipient, instructions, request_key) = (
            sender.to_string(),
            recipient.to_string(),
            instructions.to_string(),
            request_key.to_string(),
        );
        self.mutate(project, |p| {
            let team = p.teams.get(job_id).ok_or("Unknown job")?;
            if team.member(&sender).is_none() || team.member(&recipient).is_none() {
                return Err("The AI does not participate in this job".into());
            }
            if let Some(old) = p
                .tasks
                .iter()
                .find(|t| t.job_id == job_id && t.sender == sender && t.request_key == request_key)
            {
                if old.recipient != recipient
                    || old.instructions != instructions
                    || old.kind != kind
                {
                    return Err(
                        "The same request_key already belongs to another instruction".into(),
                    );
                }
                return Ok(old.clone());
            }
            let pending = p
                .tasks
                .iter()
                .filter(|t| t.job_id == job_id && t.stage.is_pending())
                .count();
            if pending >= MAX_PENDING_TASKS {
                return Err("Prototype limit: ten pending tasks".into());
            }
            let now = now_ms();
            let task = Task {
                id: uuid::Uuid::new_v4().to_string(),
                job_id: job_id.to_string(),
                sender,
                recipient,
                request_key,
                instructions,
                kind,
                stage: TaskStage::Queued,
                outcome: None,
                result: None,
                delivery_result: None,
                error: None,
                created_ms: now,
                updated_ms: now,
            };
            p.tasks.push(task.clone());
            Ok(task)
        })
    }

    /// `task_status`: a task is visible only to its sender and recipient.
    pub fn task_for(&self, project: &str, task_id: &str, caller: &str) -> Result<Task, CallError> {
        let found = self.read(project, |p| {
            p.tasks.iter().find(|t| t.id == task_id).cloned()
        })?;
        match found {
            Some(t) if t.sender == caller || t.recipient == caller => Ok(t),
            _ => Err(CallError::Tool("Task unavailable for this session".into())),
        }
    }

    pub fn tasks(&self, project: &str, job_id: &str) -> AppResult<Vec<Task>> {
        self.read(project, |p| {
            p.tasks
                .iter()
                .filter(|t| t.job_id == job_id)
                .cloned()
                .collect()
        })
    }

    /// Worker-side transition. Only the allow-listed fields change.
    pub fn update_task(&self, project: &str, task_id: &str, patch: TaskPatch) -> AppResult<Task> {
        self.mutate(project, |p| {
            let task = p
                .tasks
                .iter_mut()
                .find(|t| t.id == task_id)
                .ok_or("Task unavailable")?;
            if let Some(s) = patch.stage {
                task.stage = s;
            }
            if let Some(o) = patch.outcome {
                task.outcome = Some(o);
            }
            if let Some(r) = patch.result {
                task.result = Some(r);
            }
            if let Some(e) = patch.error {
                task.error = Some(e);
            }
            task.updated_ms = now_ms();
            Ok(task.clone())
        })
        .map_err(CallError::into_app)
    }

    /// The sender received the result and answered: delivery is complete.
    pub fn complete_return(&self, project: &str, task_id: &str, text: &str) -> AppResult<Task> {
        let text = text.to_string();
        self.mutate(project, |p| {
            let task = p
                .tasks
                .iter_mut()
                .find(|t| t.id == task_id)
                .ok_or("Task unavailable")?;
            task.stage = TaskStage::Delivered;
            task.delivery_result = Some(text);
            task.updated_ms = now_ms();
            Ok(task.clone())
        })
        .map_err(CallError::into_app)
    }

    // ----- questions ------------------------------------------------------

    /// `ask_user`: record the question and let the asker end its turn.
    pub fn ask_user(
        &self,
        project: &str,
        job_id: &str,
        sender: &str,
        body: &str,
        options: Option<&Value>,
    ) -> Result<Question, CallError> {
        let body = body.trim();
        if !(1..=MAX_INSTRUCTIONS).contains(&body.chars().count()) {
            return Err(CallError::Tool("Invalid message".into()));
        }
        let options = normalize_question_options(options).map_err(CallError::Tool)?;
        let (sender, body) = (sender.to_string(), body.to_string());
        self.mutate(project, |p| {
            let team = p
                .teams
                .get(job_id)
                .ok_or("The question requires a managed job")?;
            if team.member(&sender).is_none() {
                return Err("The AI does not participate in this job".into());
            }
            let q = Question {
                id: uuid::Uuid::new_v4().to_string(),
                job_id: job_id.to_string(),
                sender,
                body,
                options,
                status: QuestionStatus::Waiting,
                answer: None,
                created_ms: now_ms(),
            };
            p.questions.push(q.clone());
            Ok(q)
        })
    }

    pub fn questions(&self, project: &str, job_id: &str) -> AppResult<Vec<Question>> {
        self.read(project, |p| {
            p.questions
                .iter()
                .filter(|q| q.job_id == job_id)
                .cloned()
                .collect()
        })
    }

    /// User-side answer: a `choice_id` must name one of the question's
    /// options (its label becomes the body); otherwise free text is required.
    /// At most one answer per question.
    pub fn answer_question(
        &self,
        project: &str,
        question_id: &str,
        body: &str,
        choice_id: Option<&str>,
    ) -> Result<Question, CallError> {
        let body = body.trim().to_string();
        let choice_id = choice_id
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
        self.mutate(project, |p| {
            let q = p
                .questions
                .iter_mut()
                .find(|q| q.id == question_id)
                .ok_or("Unknown question")?;
            if q.status == QuestionStatus::Answered {
                return Err("This question already has a submitted response.".into());
            }
            let answer_body = match &choice_id {
                Some(c) => q
                    .options
                    .iter()
                    .find(|o| &o.id == c)
                    .map(|o| o.label.clone())
                    .ok_or("Invalid option")?,
                None => {
                    if !(1..=MAX_INSTRUCTIONS).contains(&body.chars().count()) {
                        return Err("Invalid message".into());
                    }
                    body
                }
            };
            q.status = QuestionStatus::Answered;
            q.answer = Some(Answer {
                body: answer_body,
                choice_id,
                answered_ms: now_ms(),
                delivered: false,
            });
            Ok(q.clone())
        })
    }

    /// Worker-side: the answer reached the asker's turn.
    pub fn mark_answer_delivered(&self, project: &str, question_id: &str) -> AppResult<()> {
        self.mutate(project, |p| {
            let q = p
                .questions
                .iter_mut()
                .find(|q| q.id == question_id)
                .ok_or("Unknown question")?;
            if let Some(a) = q.answer.as_mut() {
                a.delivered = true;
            }
            Ok(())
        })
        .map_err(CallError::into_app)
    }

    // ----- reviews --------------------------------------------------------

    /// `submit_review`: only a team member holding `reviewer` may record one.
    pub fn submit_review(
        &self,
        project: &str,
        job_id: &str,
        caller: &str,
        verdict: &str,
        body: &str,
    ) -> Result<Review, CallError> {
        let verdict = match verdict {
            "approved" => Verdict::Approved,
            "changes" => Verdict::Changes,
            _ => return Err(CallError::Tool("Invalid review".into())),
        };
        if !(1..=MAX_INSTRUCTIONS).contains(&body.trim().chars().count()) {
            return Err(CallError::Tool("Invalid review".into()));
        }
        let (caller, body) = (caller.to_string(), body.to_string());
        self.mutate(project, |p| {
            let team = p
                .teams
                .get(job_id)
                .filter(|t| t.has_role(&caller, "reviewer"))
                .ok_or("Only the reviewer AI can record a review")?;
            let review = Review {
                id: uuid::Uuid::new_v4().to_string(),
                job_id: job_id.to_string(),
                participant: caller,
                revision: team.revision.clone(),
                verdict,
                body,
                created_ms: now_ms(),
            };
            p.reviews.push(review.clone());
            Ok(review)
        })
    }

    pub fn reviews(&self, project: &str, job_id: &str) -> AppResult<Vec<Review>> {
        self.read(project, |p| {
            p.reviews
                .iter()
                .filter(|r| r.job_id == job_id)
                .cloned()
                .collect()
        })
    }
}

// ---------------------------------------------------------------------------
// MCP dispatcher — port of `mcp/server.py::call`
// ---------------------------------------------------------------------------

/// A tool call either fails for the agent (`Tool`, returned as an MCP
/// `isError` result it can read and act on) or for us (`Io`).
#[derive(Debug)]
pub enum CallError {
    Tool(String),
    Io(AppError),
}

impl From<AppError> for CallError {
    fn from(e: AppError) -> Self {
        Self::Io(e)
    }
}

#[allow(dead_code)]
impl CallError {
    fn into_app(self) -> AppError {
        match self {
            Self::Tool(m) => AppError::InvalidArgument(m),
            Self::Io(e) => e,
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::Tool(m) => m.clone(),
            Self::Io(e) => e.to_string(),
        }
    }
}

/// Tool names the dispatcher serves. The bridge's `tools/list` advertises
/// exactly these (phase 2) and validates argument shapes before relaying;
/// the dispatcher re-checks what matters semantically.
pub const TOOL_NAMES: &[&str] = &[
    "list_participants",
    "delegate_task",
    "task_status",
    "ask_user",
    "submit_review",
    "send_message",
    "create_task",
];

/// One relayed `tools/call`, as the bridge posts it to `/mcp`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCall {
    /// Project root the room belongs to (keys the persisted state).
    pub project: String,
    /// Room id.
    pub job: String,
    /// Participant id of the agent making the call.
    pub caller: String,
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, CallError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| CallError::Tool("Unknown or incomplete arguments".into()))
}

impl ConnectorService {
    /// Dispatch one tool call for `caller` inside `job`. Mirrors the
    /// per-tool branches of ai-connector's `call`, minus the ones phase 1
    /// does not ship (`send_message`, `create_task`, `start_work`,
    /// `claim_files`/`release_files`).
    pub fn call(&self, req: &McpCall) -> Result<Value, CallError> {
        if !TOOL_NAMES.contains(&req.name.as_str()) {
            return Err(CallError::Tool("Unknown tool".into()));
        }
        let args = &req.arguments;
        if !args.is_object() && !args.is_null() {
            return Err(CallError::Tool("Invalid arguments".into()));
        }
        // The caller must be on the job's team before it may do anything —
        // a stray process with the token still learns nothing about a room
        // it is not part of.
        let team = self
            .team(&req.project, &req.job)?
            .ok_or_else(|| CallError::Tool("Unknown job".into()))?;
        if team.member(&req.caller).is_none() {
            return Err(CallError::Tool(
                "The AI does not participate in this job".into(),
            ));
        }
        match req.name.as_str() {
            "list_participants" => Ok(json!({
                "participants": team.members.iter().map(|m| json!({
                    "id": m.id,
                    "role": m.roles.first().cloned().unwrap_or_else(|| "assistant".into()),
                    "roles": m.roles,
                })).collect::<Vec<_>>()
            })),
            "delegate_task" => {
                let recipient = arg_str(args, "recipient")?;
                if team.member(recipient).is_none() {
                    return Err(CallError::Tool(
                        "Use the participant ID returned by list_participants".into(),
                    ));
                }
                let task = self.delegate(
                    &req.project,
                    &req.job,
                    &req.caller,
                    recipient,
                    arg_str(args, "instructions")?,
                    arg_str(args, "request_key")?,
                    TaskKind::Task,
                )?;
                Ok(json!({ "id": task.id, "stage": task.stage, "request_key": task.request_key }))
            }
            "send_message" => {
                let recipient = arg_str(args, "recipient")?;
                if team.member(recipient).is_none() {
                    return Err(CallError::Tool(
                        "Use the participant ID returned by list_participants".into(),
                    ));
                }
                let kind = match args.get("kind") {
                    None | Some(Value::Null) => TaskKind::Consult,
                    Some(Value::String(k)) if k == "consult" => TaskKind::Consult,
                    Some(Value::String(k)) if k == "discussion" => TaskKind::Discussion,
                    Some(_) => return Err(CallError::Tool("Invalid communication type".into())),
                };
                let task = self.delegate(
                    &req.project,
                    &req.job,
                    &req.caller,
                    recipient,
                    arg_str(args, "body")?,
                    arg_str(args, "request_key")?,
                    kind,
                )?;
                Ok(json!({ "id": task.id, "stage": task.stage, "kind": task.kind }))
            }
            "create_task" => {
                let job = self.create_task(
                    &req.project,
                    &req.job,
                    &req.caller,
                    arg_str(args, "instructions")?,
                    arg_str(args, "request_key")?,
                )?;
                Ok(json!({ "job_id": job.id, "status": job.status }))
            }
            "task_status" => {
                let task = self.task_for(&req.project, arg_str(args, "task_id")?, &req.caller)?;
                Ok(json!({
                    "id": task.id, "stage": task.stage, "outcome": task.outcome,
                    "result": task.result, "error": task.error,
                }))
            }
            "ask_user" => {
                let q = self.ask_user(
                    &req.project,
                    &req.job,
                    &req.caller,
                    arg_str(args, "question")?,
                    args.get("options"),
                )?;
                Ok(
                    json!({ "id": q.id, "status": q.status, "question": q.body, "options": q.options }),
                )
            }
            "submit_review" => {
                let review = self.submit_review(
                    &req.project,
                    &req.job,
                    &req.caller,
                    arg_str(args, "verdict")?,
                    arg_str(args, "body")?,
                )?;
                Ok(json!({ "verdict": review.verdict, "revision": review.revision }))
            }
            _ => Err(CallError::Tool("Unknown tool".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(job: &str) -> Team {
        Team {
            job_id: job.into(),
            members: vec![
                TeamMember {
                    id: "p1".into(),
                    roles: vec!["organizer".into()],
                },
                TeamMember {
                    id: "p2".into(),
                    roles: vec!["implementer".into()],
                },
                TeamMember {
                    id: "p3".into(),
                    roles: vec!["reviewer".into()],
                },
            ],
            revision: "initial".into(),
        }
    }

    fn call(name: &str, caller: &str, args: Value) -> McpCall {
        McpCall {
            project: "/proj/a".into(),
            job: "room-1".into(),
            caller: caller.into(),
            name: name.into(),
            arguments: args,
        }
    }

    fn tool_err(r: Result<Value, CallError>) -> String {
        match r {
            Err(CallError::Tool(m)) => m,
            other => panic!("expected a tool error, got {other:?}"),
        }
    }

    /// One test fn: it owns the process-global XDG_DATA_HOME while it runs.
    #[test]
    fn connector_contract_and_persistence() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "ac-connector-test-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = ConnectorService::new();
        let proj = "/proj/a";

        // Fresh install: nothing known, and asking is not an error.
        assert!(svc.team(proj, "room-1").unwrap().is_none());
        assert_eq!(
            tool_err(svc.call(&call("list_participants", "p1", json!({})))),
            "Unknown job"
        );

        // Team rules: invalid roles / duplicate ids are refused.
        let mut bad = team("room-1");
        bad.members[0].roles = vec!["boss".into()];
        assert!(svc.register_team(proj, bad).is_err());
        let mut dup = team("room-1");
        dup.members[1].id = "p1".into();
        assert!(svc.register_team(proj, dup).is_err());
        svc.register_team(proj, team("room-1")).unwrap();

        // Outsiders learn nothing.
        assert_eq!(
            tool_err(svc.call(&call("list_participants", "zz", json!({})))),
            "The AI does not participate in this job"
        );
        assert_eq!(
            tool_err(svc.call(&call("nope", "p1", json!({})))),
            "Unknown tool"
        );

        // list_participants: opaque ids and roles only.
        let listed = svc
            .call(&call("list_participants", "p1", json!({})))
            .unwrap();
        let parts = listed["participants"].as_array().unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2]["role"], "reviewer");
        assert_eq!(parts[2]["roles"], json!(["reviewer"]));

        // delegate_task: happy path, then the request_key contract.
        let args =
            json!({ "recipient": "p2", "instructions": "Read docs/x.md", "request_key": "k1" });
        let first = svc
            .call(&call("delegate_task", "p1", args.clone()))
            .unwrap();
        assert_eq!(first["stage"], "queued");
        let again = svc.call(&call("delegate_task", "p1", args)).unwrap();
        assert_eq!(
            again["id"], first["id"],
            "same request → same task, executed once"
        );
        assert_eq!(
            tool_err(svc.call(&call(
                "delegate_task",
                "p1",
                json!({ "recipient": "p2", "instructions": "Something else", "request_key": "k1" })
            ))),
            "The same request_key already belongs to another instruction"
        );
        assert_eq!(
            tool_err(svc.call(&call(
                "delegate_task",
                "p1",
                json!({ "recipient": "p1", "instructions": "Self", "request_key": "k2" })
            ))),
            "Invalid delegation: another participant in the same project is required"
        );
        assert_eq!(
            tool_err(svc.call(&call(
                "delegate_task",
                "p1",
                json!({ "recipient": "Codex", "instructions": "By name", "request_key": "k3" })
            ))),
            "Use the participant ID returned by list_participants"
        );
        assert_eq!(
            tool_err(svc.call(&call(
                "delegate_task",
                "p1",
                json!({ "recipient": "p2", "instructions": "   ", "request_key": "k4" })
            ))),
            "Invalid instructions"
        );
        assert_eq!(
            tool_err(svc.call(&call("delegate_task", "p1", json!({ "recipient": "p2" })))),
            "Unknown or incomplete arguments"
        );

        // task_status: visible to sender and recipient, nobody else.
        let task_id = first["id"].as_str().unwrap().to_string();
        let st = svc
            .call(&call("task_status", "p2", json!({ "task_id": task_id })))
            .unwrap();
        assert_eq!(st["stage"], "queued");
        assert!(st["result"].is_null());
        assert_eq!(
            tool_err(svc.call(&call("task_status", "p3", json!({ "task_id": task_id })))),
            "Task unavailable for this session"
        );

        // Worker transitions and the pending limit.
        svc.update_task(
            proj,
            &task_id,
            TaskPatch {
                stage: Some(TaskStage::Executing),
                ..Default::default()
            },
        )
        .unwrap();
        svc.update_task(
            proj,
            &task_id,
            TaskPatch {
                stage: Some(TaskStage::Ready),
                outcome: Some(Outcome::Succeeded),
                result: Some("Title: X".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let st = svc
            .call(&call("task_status", "p1", json!({ "task_id": task_id })))
            .unwrap();
        assert_eq!(st["outcome"], "succeeded");
        assert_eq!(st["result"], "Title: X");
        for i in 0..9 {
            svc.call(&call(
                "delegate_task",
                "p1",
                json!({ "recipient": "p2", "instructions": format!("job {i}"), "request_key": format!("fill-{i}") }),
            ))
            .unwrap();
        }
        assert_eq!(
            tool_err(svc.call(&call(
                "delegate_task",
                "p1",
                json!({ "recipient": "p2", "instructions": "one more", "request_key": "overflow" })
            ))),
            "Prototype limit: ten pending tasks"
        );
        let delivered = svc
            .complete_return(proj, &task_id, "Thanks, noted.")
            .unwrap();
        assert_eq!(delivered.stage, TaskStage::Delivered);
        assert_eq!(delivered.delivery_result.as_deref(), Some("Thanks, noted."));
        // The slot is free again.
        svc.call(&call(
            "delegate_task",
            "p1",
            json!({ "recipient": "p2", "instructions": "one more", "request_key": "overflow" }),
        ))
        .unwrap();
        assert_eq!(svc.tasks(proj, "room-1").unwrap().len(), 11);

        // ask_user: options contract (ids generated, duplicates refused, caps).
        let q = svc
            .call(&call(
                "ask_user",
                "p1",
                json!({ "question": "Which scope?", "options": [
                    { "id": "docs", "label": "Documentation only", "description": "No code changes" },
                    { "label": "Change code" },
                    "Both"
                ]}),
            ))
            .unwrap();
        assert_eq!(q["status"], "waiting");
        let opts = q["options"].as_array().unwrap();
        assert_eq!(
            opts.iter()
                .map(|o| o["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["docs", "opt-2", "opt-3"]
        );
        assert_eq!(opts[0]["description"], "No code changes");
        assert_eq!(opts[2]["label"], "Both");
        let legacy = svc
            .call(&call("ask_user", "p1", json!({ "question": "Plain?" })))
            .unwrap();
        assert_eq!(legacy["options"], json!([]));
        let bad_options = [
            (
                json!({ "question": "q", "options": "no" }),
                "Invalid question options",
            ),
            (
                json!({ "question": "q", "options": [1] }),
                "Invalid question option",
            ),
            (
                json!({ "question": "q", "options": [{ "label": "a", "extra": 1 }] }),
                "Invalid question option",
            ),
            (
                json!({ "question": "q", "options": [{ "label": "" }] }),
                "Invalid option label",
            ),
            (
                json!({ "question": "q", "options": [{ "id": "has space", "label": "a" }] }),
                "Invalid option identifier",
            ),
            (
                json!({ "question": "q", "options": [{ "label": "a", "description": "d".repeat(301) }] }),
                "Invalid option description",
            ),
            (
                json!({ "question": "q", "options": ["a", "a"] }),
                "Duplicate question options",
            ),
            (
                json!({ "question": "q", "options": [{ "id": "x", "label": "a" }, { "id": "x", "label": "b" }] }),
                "Duplicate question options",
            ),
            (
                json!({ "question": "q", "options": ["1", "2", "3", "4", "5", "6", "7"] }),
                "A question supports up to 6 options",
            ),
            (json!({ "question": "  " }), "Invalid message"),
        ];
        for (args, expected) in bad_options {
            assert_eq!(
                tool_err(svc.call(&call("ask_user", "p1", args.clone()))),
                expected,
                "{args}"
            );
        }

        // Answering: a choice yields the label; a second answer is refused.
        let qid = q["id"].as_str().unwrap();
        assert_eq!(
            svc.answer_question(proj, qid, "", Some("nope"))
                .err()
                .map(|e| e.message()),
            Some("Invalid option".into())
        );
        let answered = svc.answer_question(proj, qid, "", Some("docs")).unwrap();
        assert_eq!(answered.answer.as_ref().unwrap().body, "Documentation only");
        assert_eq!(answered.status, QuestionStatus::Answered);
        assert_eq!(
            svc.answer_question(proj, qid, "again", None)
                .err()
                .map(|e| e.message()),
            Some("This question already has a submitted response.".into())
        );
        let lid = legacy["id"].as_str().unwrap();
        assert_eq!(
            svc.answer_question(proj, lid, "  ", None)
                .err()
                .map(|e| e.message()),
            Some("Invalid message".into())
        );
        let free = svc
            .answer_question(proj, lid, "Only the API", None)
            .unwrap();
        assert_eq!(free.answer.as_ref().unwrap().body, "Only the API");
        assert!(!free.answer.as_ref().unwrap().delivered);
        svc.mark_answer_delivered(proj, lid).unwrap();
        let qs = svc.questions(proj, "room-1").unwrap();
        assert!(
            qs.iter()
                .find(|x| x.id == lid)
                .unwrap()
                .answer
                .as_ref()
                .unwrap()
                .delivered
        );

        // send_message: a consultation rides the same queue with its kind. Free
        // two slots first: the queue is still at the ten-pending limit.
        for key in ["fill-0", "fill-1"] {
            let id = svc
                .tasks(proj, "room-1")
                .unwrap()
                .into_iter()
                .find(|t| t.request_key == key)
                .map(|t| t.id)
                .unwrap();
            svc.complete_return(proj, &id, "ok").unwrap();
        }
        let c = svc
            .call(&call(
                "send_message",
                "p2",
                json!({ "recipient": "p1", "body": "Which branch do we target?", "request_key": "c1" }),
            ))
            .unwrap();
        assert_eq!(c["kind"], "consult");
        assert_eq!(c["stage"], "queued");
        let d = svc
            .call(&call(
                "send_message",
                "p2",
                json!({ "recipient": "p1", "body": "Let's argue scope", "request_key": "c2", "kind": "discussion" }),
            ))
            .unwrap();
        assert_eq!(d["kind"], "discussion");
        assert_eq!(
            tool_err(svc.call(&call(
                "send_message",
                "p2",
                json!({ "recipient": "p1", "body": "x", "request_key": "c3", "kind": "review" })
            ))),
            "Invalid communication type"
        );
        let consult_tasks: Vec<_> = svc
            .tasks(proj, "room-1")
            .unwrap()
            .into_iter()
            .filter(|t| t.kind == TaskKind::Consult)
            .collect();
        assert_eq!(consult_tasks.len(), 1);
        assert_eq!(consult_tasks[0].instructions, "Which branch do we target?");

        // create_task: a split-off job waits for the human; idempotent per key.
        let pj = svc
            .call(&call(
                "create_task",
                "p1",
                json!({ "instructions": "Later: add tests for the parser", "request_key": "later-1" }),
            ))
            .unwrap();
        assert_eq!(pj["status"], "pending_approval");
        let again = svc
            .call(&call(
                "create_task",
                "p1",
                json!({ "instructions": "Later: add tests for the parser", "request_key": "later-1" }),
            ))
            .unwrap();
        assert_eq!(again["job_id"], pj["job_id"]);
        assert_eq!(
            tool_err(svc.call(&call(
                "create_task",
                "p1",
                json!({ "instructions": "Something else", "request_key": "later-1" })
            ))),
            "The same request_key already belongs to another instruction"
        );
        for i in 0..9 {
            svc.call(&call(
                "create_task",
                "p1",
                json!({ "instructions": format!("later {i}"), "request_key": format!("fill-later-{i}") }),
            ))
            .unwrap();
        }
        assert_eq!(
            tool_err(svc.call(&call(
                "create_task",
                "p1",
                json!({ "instructions": "one too many", "request_key": "later-overflow" })
            ))),
            "Each job may have at most 10 tasks waiting for approval"
        );
        let pending_id = pj["job_id"].as_str().unwrap();
        let resolved = svc
            .resolve_pending(proj, pending_id, Some("r-new"))
            .unwrap();
        assert_eq!(resolved.status, PendingStatus::Approved);
        assert_eq!(resolved.approved_job_id.as_deref(), Some("r-new"));
        assert!(
            svc.resolve_pending(proj, pending_id, None).is_err(),
            "already resolved"
        );
        assert_eq!(svc.pending_jobs(proj, "room-1").unwrap().len(), 10);
        // An approval frees a slot.
        svc.call(&call(
            "create_task",
            "p1",
            json!({ "instructions": "one too many", "request_key": "later-overflow" }),
        ))
        .unwrap();

        // submit_review: reviewer only, verdict vocabulary, revision label.
        assert_eq!(
            tool_err(svc.call(&call(
                "submit_review",
                "p2",
                json!({ "verdict": "approved", "body": "ok" })
            ))),
            "Only the reviewer AI can record a review"
        );
        assert_eq!(
            tool_err(svc.call(&call(
                "submit_review",
                "p3",
                json!({ "verdict": "meh", "body": "ok" })
            ))),
            "Invalid review"
        );
        assert_eq!(
            tool_err(svc.call(&call(
                "submit_review",
                "p3",
                json!({ "verdict": "changes", "body": " " })
            ))),
            "Invalid review"
        );
        let r = svc
            .call(&call(
                "submit_review",
                "p3",
                json!({ "verdict": "changes", "body": "Missing tests" }),
            ))
            .unwrap();
        assert_eq!(r, json!({ "verdict": "changes", "revision": "initial" }));
        svc.set_revision(proj, "room-1", "turn-7").unwrap();
        let r = svc
            .call(&call(
                "submit_review",
                "p3",
                json!({ "verdict": "approved", "body": "LGTM" }),
            ))
            .unwrap();
        assert_eq!(r["revision"], "turn-7");
        assert_eq!(svc.reviews(proj, "room-1").unwrap().len(), 2);
        // Re-registering the team (every driver start) keeps the revision.
        svc.register_team(proj, team("room-1")).unwrap();
        assert_eq!(
            svc.team(proj, "room-1").unwrap().unwrap().revision,
            "turn-7"
        );

        // Persistence: atomic write, backup, per-project isolation, corrupt
        // main file recovers from .bak, forget_job clears the job.
        let path = ConnectorService::path().unwrap();
        let bak = ConnectorService::bak_path().unwrap();
        let tmp = ConnectorService::tmp_path().unwrap();
        assert!(path.exists() && bak.exists() && !tmp.exists());
        svc.register_team("/proj/b", team("room-9")).unwrap();
        assert_eq!(
            svc.tasks(proj, "room-1").unwrap().len(),
            13,
            "proj a untouched by proj b"
        );
        let good = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, "{ not valid json ").unwrap();
        std::fs::write(&bak, &good).unwrap();
        assert_eq!(
            svc.tasks(proj, "room-1").unwrap().len(),
            13,
            "recovered from .bak"
        );
        std::fs::write(&path, &good).unwrap();
        svc.forget_job(proj, "room-1").unwrap();
        assert!(svc.team(proj, "room-1").unwrap().is_none());
        assert!(svc.tasks(proj, "room-1").unwrap().is_empty());
        assert!(svc.questions(proj, "room-1").unwrap().is_empty());
        assert!(svc.pending_jobs(proj, "room-1").unwrap().is_empty());
        assert!(svc.team("/proj/b", "room-9").unwrap().is_some());

        std::env::remove_var("XDG_DATA_HOME");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn retention_never_drops_pending_work() {
        let mut p = ProjectConnector::default();
        for i in 0..(KEEP_TASKS + 5) {
            p.tasks.push(Task {
                id: format!("t{i}"),
                job_id: "j".into(),
                sender: "p1".into(),
                recipient: "p2".into(),
                request_key: format!("k{i}"),
                instructions: "x".into(),
                kind: TaskKind::Task,
                // The first ten are finished; everything after is pending.
                stage: if i < 10 {
                    TaskStage::Delivered
                } else {
                    TaskStage::Queued
                },
                outcome: None,
                result: None,
                delivery_result: None,
                error: None,
                created_ms: i as u64,
                updated_ms: i as u64,
            });
        }
        p.trim();
        // Only finished tasks went (the five oldest), pending ones all stayed.
        assert_eq!(p.tasks.len(), KEEP_TASKS);
        assert!(
            p.tasks
                .iter()
                .filter(|t| t.stage == TaskStage::Delivered)
                .count()
                == 5
        );
        assert_eq!(
            p.tasks.iter().filter(|t| t.stage.is_pending()).count(),
            KEEP_TASKS - 5
        );
    }
}
