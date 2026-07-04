//! Visual scheduler: durable, suggest-only agentic jobs on a clock.
//!
//! A "job" pairs a *trigger* (interval / daily / weekly / event) with an
//! *action* (run a skill, a free prompt, or a small pipeline of those). When a
//! job fires, the action runs through `claude -p --permission-mode plan` — the
//! SAME plan-mode enforcement the Advisor and the learning curator use, so a
//! scheduled run physically *cannot* mutate the repo. Its textual output is
//! captured as a reviewable result in a per-project run-history ledger; the user
//! decides what (if anything) to materialize. That is the "suggest-only"
//! guarantee, baked into how the job runs rather than promised by convention.
//!
//! Persistence mirrors the proven patterns already in this crate:
//! - jobs live in a single crash-safe `scheduler.json` (temp-write + `.bak` +
//!   rename), keyed by project — exactly like `sessions_service`;
//! - run history is an append-only JSONL ledger per project with a size cap —
//!   exactly like `activity_service`.
//!
//! The tick loop is a background thread started from `lib.rs` setup, the same
//! shape as `hooks_service::start_watcher`, and emits `scheduler://*` events the
//! frontend listens to.
//!
//! Time math is done purely in UTC from epoch milliseconds (no `chrono`/`time`
//! dependency). Daily/weekly `hour`/`minute` are therefore interpreted as UTC;
//! the frontend converts to/from the user's local zone for display and editing.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use parking_lot::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

const DAY_MS: u64 = 86_400_000;
/// How often the tick loop wakes to run due jobs. Daily/weekly/interval jobs
/// don't need sub-minute precision, so a calm 30s poll keeps idle cost near zero.
const TICK_INTERVAL: Duration = Duration::from_secs(30);
/// History ledger cap (bytes) before it's trimmed to the most recent runs.
const HISTORY_MAX_BYTES: u64 = 4 * 1024 * 1024;
const HISTORY_KEEP: usize = 1000;
/// Cap on a stored run-output excerpt so one chatty run can't bloat the ledger.
const OUTPUT_EXCERPT_MAX: usize = 4000;
/// Exponential-backoff base after a failed run (doubles per consecutive failure).
const BACKOFF_BASE_MS: u64 = 5 * 60_000;
/// Backoff ceiling, so a permanently-broken job retries at most this often.
const BACKOFF_MAX_MS: u64 = 6 * 60 * 60_000;

/// What makes a job fire. Time-based variants compute a concrete `next_due`;
/// `Event` is inert in the time loop (Phase 3 wires it to the hook stream) and
/// only runs via an explicit "run now".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    /// Every `every_ms` from the last run (or from creation if never run).
    Interval { every_ms: u64 },
    /// Daily at `hour`:`minute` (UTC).
    Daily { hour: u32, minute: u32 },
    /// Weekly on `weekday` (0=Sunday..6=Saturday, JS getUTCDay convention) at
    /// `hour`:`minute` (UTC).
    Weekly { weekday: u32, hour: u32, minute: u32 },
    /// Fires when a named app event arrives (e.g. "corpus_grew"). Inert until
    /// Phase 3; stored and editable now.
    Event { name: String },
}

/// What a job does when it fires. Every leaf runs through plan-mode `claude`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    /// Invoke a slash-command / skill, optionally with trailing args.
    Skill {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<String>,
    },
    /// Send a free-form prompt.
    Prompt { text: String },
    /// Run a sequence of conditional steps (see `PipelineStep`).
    Pipeline { steps: Vec<PipelineStep> },
}

/// One step of a pipeline: an action plus an optional condition gating it on the
/// previous executed step's result. With no condition a step runs only if the
/// prior step succeeded (a dependent chain — an error halts the rest), which is
/// the natural default; explicit conditions let a pipeline branch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStep {
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<StepCondition>,
}

/// A gate evaluated against the previous executed step's (status, output).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StepCondition {
    /// Run only if the prior output contains `text` (case-insensitive).
    Contains { text: String },
    /// Run only if the prior step errored (a recovery/branch step).
    PrevFailed,
    /// Run only if the prior step succeeded.
    PrevOk,
}

/// What to do with a firing that was missed because the app was closed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum OnMissed {
    /// Run it once on next launch (don't replay every missed interval).
    #[default]
    Catchup,
    /// Skip the missed firing, just reschedule to the next future slot.
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub action: Action,
    #[serde(default)]
    pub on_missed: OnMissed,
    /// Minimum gap between runs (guards against run-now spam racing the tick).
    #[serde(default)]
    pub cooldown_ms: u64,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_due_ms: Option<u64>,
    /// Consecutive failed runs; drives exponential backoff. Reset to 0 on success.
    #[serde(default)]
    pub consecutive_failures: u32,
    /// While set and in the future, the job is held back (backoff after errors),
    /// independent of its trigger — so a broken job can't hammer `claude` (or
    /// spend tokens) every tick/event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_until_ms: Option<u64>,
}

/// One recorded execution, appended to the per-project history ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub job_id: String,
    pub job_name: String,
    pub started_ms: u64,
    pub finished_ms: u64,
    /// "ok" | "error" | "missed"
    pub status: String,
    pub summary: String,
    pub output_excerpt: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SchedulerFile {
    #[serde(default)]
    by_project: HashMap<String, Vec<Job>>,
}

pub struct SchedulerService {
    /// Guards the jobs file (read-modify-write).
    lock: Mutex<()>,
    /// Serializes job execution to a global concurrency cap of 1, so the tick
    /// loop and an explicit run-now never run two agents at once.
    run_lock: Mutex<()>,
    started: Mutex<bool>,
    /// Global kill-switch: when true, the tick loop and event firing run nothing
    /// (an explicit run-now still works — it's a deliberate manual override).
    /// Cached here and backed by `scheduler-config.json` so it survives restart.
    paused: AtomicBool,
}

impl Default for SchedulerService {
    fn default() -> Self {
        Self::new()
    }
}

impl SchedulerService {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            run_lock: Mutex::new(()),
            started: Mutex::new(false),
            paused: AtomicBool::new(load_paused_flag()),
        }
    }

    // ---- paths -----------------------------------------------------------

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("scheduler.json"))
    }
    fn bak_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("scheduler.json.bak"))
    }
    fn tmp_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("scheduler.json.tmp"))
    }

    fn history_dir() -> AppResult<PathBuf> {
        let dir = Self::dir()?.join("scheduler-history");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Stable, human-recognizable per-project ledger filename (same scheme as
    /// the activity ledger): a cleaned basename plus a hash of the full root.
    fn history_path(project_root: &str) -> AppResult<PathBuf> {
        let mut h = DefaultHasher::new();
        project_root.hash(&mut h);
        let hash = h.finish();
        let last = project_root
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("root");
        let clean: String = last
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .take(24)
            .collect();
        Ok(Self::history_dir()?.join(format!("{clean}-{hash:016x}.jsonl")))
    }

    // ---- jobs file (crash-safe, mirrors sessions_service) ----------------

    fn load_file() -> AppResult<SchedulerFile> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(SchedulerFile::default());
        }
        let txt = fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("read scheduler.json: {e}")))?;
        if txt.trim().is_empty() {
            return Ok(SchedulerFile::default());
        }
        match serde_json::from_str::<SchedulerFile>(&txt) {
            Ok(file) => Ok(file),
            Err(e) => {
                if let Ok(bak) = Self::bak_path() {
                    if let Ok(btxt) = fs::read_to_string(&bak) {
                        if let Ok(file) = serde_json::from_str::<SchedulerFile>(&btxt) {
                            return Ok(file);
                        }
                    }
                }
                Err(AppError::Other(format!("parse scheduler.json: {e}")))
            }
        }
    }

    fn write_file(file: &SchedulerFile) -> AppResult<()> {
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

    // ---- public API (called by commands) ---------------------------------

    pub fn list(&self, project_root: &str) -> AppResult<Vec<Job>> {
        let _g = self.lock.lock();
        let file = Self::load_file()?;
        Ok(file.by_project.get(project_root).cloned().unwrap_or_default())
    }

    /// Create (or replace, if the id already exists) a job. Fills id/created_at
    /// when absent and computes the first `next_due` from the trigger.
    pub fn create(&self, project_root: &str, mut job: Job) -> AppResult<Job> {
        let _g = self.lock.lock();
        let now = now_ms();
        if job.id.trim().is_empty() {
            job.id = uuid::Uuid::new_v4().to_string();
        }
        if job.created_at_ms == 0 {
            job.created_at_ms = now;
        }
        // Schedule the first firing strictly after now.
        job.next_due_ms = compute_next_due(&job.trigger, now);
        let mut file = Self::load_file()?;
        let bucket = file.by_project.entry(project_root.to_string()).or_default();
        bucket.retain(|j| j.id != job.id);
        bucket.push(job.clone());
        Self::write_file(&file)?;
        Ok(job)
    }

    /// Update a job in place. Recomputes `next_due` from now using the (possibly
    /// changed) trigger, and preserves run history fields.
    pub fn update(&self, project_root: &str, mut job: Job) -> AppResult<Job> {
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        let bucket = file
            .by_project
            .get_mut(project_root)
            .ok_or_else(|| AppError::NotFound(format!("no jobs for project {project_root}")))?;
        let existing = bucket
            .iter()
            .find(|j| j.id == job.id)
            .ok_or_else(|| AppError::NotFound(format!("job {} not found", job.id)))?;
        job.created_at_ms = existing.created_at_ms;
        job.last_run_ms = existing.last_run_ms;
        job.next_due_ms = compute_next_due(&job.trigger, now_ms());
        bucket.retain(|j| j.id != job.id);
        bucket.push(job.clone());
        Self::write_file(&file)?;
        Ok(job)
    }

    pub fn delete(&self, project_root: &str, id: &str) -> AppResult<()> {
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        if let Some(bucket) = file.by_project.get_mut(project_root) {
            bucket.retain(|j| j.id != id);
            if bucket.is_empty() {
                file.by_project.remove(project_root);
            }
        }
        Self::write_file(&file)
    }

    /// Pause/resume a job. A re-enabled time-based job is rescheduled from now.
    pub fn set_enabled(&self, project_root: &str, id: &str, enabled: bool) -> AppResult<Job> {
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        let bucket = file
            .by_project
            .get_mut(project_root)
            .ok_or_else(|| AppError::NotFound(format!("no jobs for project {project_root}")))?;
        let job = bucket
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or_else(|| AppError::NotFound(format!("job {id} not found")))?;
        job.enabled = enabled;
        if enabled {
            job.next_due_ms = compute_next_due(&job.trigger, now_ms());
        }
        let updated = job.clone();
        Self::write_file(&file)?;
        Ok(updated)
    }

    pub fn history(&self, project_root: &str, limit: Option<usize>) -> AppResult<Vec<RunRecord>> {
        let path = Self::history_path(project_root)?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path)?;
        let mut records: Vec<RunRecord> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<RunRecord>(l).ok())
            .collect();
        records.reverse(); // newest first for the results feed
        if let Some(n) = limit {
            records.truncate(n);
        }
        Ok(records)
    }

    /// Whether the global kill-switch is engaged.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Engage/release the global kill-switch (persisted across restarts).
    pub fn set_paused(&self, app: &AppHandle, paused: bool) -> AppResult<()> {
        self.paused.store(paused, Ordering::Relaxed);
        save_paused_flag(paused)?;
        let _ = app.emit("scheduler://paused_changed", serde_json::json!({ "paused": paused }));
        Ok(())
    }

    fn append_history(project_root: &str, rec: &RunRecord) -> AppResult<()> {
        let path = Self::history_path(project_root)?;
        let mut line = serde_json::to_string(rec)
            .map_err(|e| AppError::Other(format!("serialize run record: {e}")))?;
        line.push('\n');
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        f.write_all(line.as_bytes())?;
        drop(f);
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() > HISTORY_MAX_BYTES {
                let _ = Self::trim_history(&path);
            }
        }
        Ok(())
    }

    fn trim_history(path: &Path) -> AppResult<()> {
        let content = fs::read_to_string(path)?;
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() <= HISTORY_KEEP {
            return Ok(());
        }
        let kept = lines[lines.len() - HISTORY_KEEP..].join("\n");
        let tmp = path.with_extension("jsonl.tmp");
        fs::write(&tmp, format!("{kept}\n"))?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Run a job immediately by id (UI "run now"). Honors the cap-of-1 run lock.
    pub fn run_now(&self, app: &AppHandle, project_root: &str, id: &str) -> AppResult<RunRecord> {
        let job = {
            let _g = self.lock.lock();
            Self::load_file()?
                .by_project
                .get(project_root)
                .and_then(|b| b.iter().find(|j| j.id == id).cloned())
                .ok_or_else(|| AppError::NotFound(format!("job {id} not found")))?
        };
        let rec = self.execute(app, project_root, &job);
        self.mark_ran(project_root, &job, &rec)?;
        Ok(rec)
    }

    /// Fire all enabled jobs whose trigger is `Event { name }` matching `name`,
    /// for the given project. Selection (and cooldown gating) happens up front;
    /// the actual runs are offloaded to a thread so the caller — a hook handler
    /// or a command — never blocks on `claude`. Runs serialize through the same
    /// run lock as the tick loop (cap of 1).
    pub fn fire_event(&self, app: &AppHandle, project_root: &str, name: &str) {
        if self.is_paused() {
            return;
        }
        let now = now_ms();
        let jobs = match self.list(project_root) {
            Ok(j) => j,
            Err(_) => return,
        };
        let due: Vec<Job> = jobs
            .into_iter()
            .filter(|job| {
                job.enabled
                    && matches!(&job.trigger, Trigger::Event { name: n } if n == name)
                    && cooldown_ok(job, now)
                    && backoff_ok(job, now)
            })
            .collect();
        if due.is_empty() {
            return;
        }
        let project = project_root.to_string();
        let app2 = app.clone();
        thread::spawn(move || {
            let state = app2.state::<AppState>();
            let sched = &state.scheduler;
            for job in &due {
                let rec = sched.execute(&app2, &project, job);
                let _ = sched.mark_ran(&project, job, &rec);
            }
            let _ = app2.emit("scheduler://jobs_changed", serde_json::json!({}));
        });
    }

    // ---- execution -------------------------------------------------------

    /// Run a job's action under the global run lock, record + emit the result.
    fn execute(&self, app: &AppHandle, project_root: &str, job: &Job) -> RunRecord {
        let _run = self.run_lock.lock();
        let started = now_ms();
        let _ = app.emit(
            "scheduler://run_started",
            serde_json::json!({ "jobId": job.id, "jobName": job.name, "startedMs": started }),
        );
        let (status, output) = run_action(Path::new(project_root), &job.action);
        let finished = now_ms();
        let rec = RunRecord {
            job_id: job.id.clone(),
            job_name: job.name.clone(),
            started_ms: started,
            finished_ms: finished,
            status,
            summary: summarize(&output),
            output_excerpt: truncate(&output, OUTPUT_EXCERPT_MAX),
        };
        let _ = Self::append_history(project_root, &rec);
        let _ = app.emit("scheduler://run_finished", &rec);
        rec
    }

    /// After a run, stamp last_run, reschedule the next firing, and update the
    /// failure/backoff state: an error grows the backoff window exponentially; a
    /// success clears it.
    fn mark_ran(&self, project_root: &str, job: &Job, rec: &RunRecord) -> AppResult<()> {
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        if let Some(bucket) = file.by_project.get_mut(project_root) {
            if let Some(j) = bucket.iter_mut().find(|j| j.id == job.id) {
                j.last_run_ms = Some(rec.started_ms);
                j.next_due_ms = compute_next_due(&j.trigger, rec.started_ms);
                if rec.status == "error" {
                    j.consecutive_failures = j.consecutive_failures.saturating_add(1);
                    j.backoff_until_ms =
                        Some(rec.started_ms.saturating_add(backoff_delay(j.consecutive_failures)));
                } else {
                    j.consecutive_failures = 0;
                    j.backoff_until_ms = None;
                }
            }
        }
        Self::write_file(&file)?;
        Ok(())
    }

    fn record_missed(&self, project_root: &str, job: &Job) {
        let now = now_ms();
        let rec = RunRecord {
            job_id: job.id.clone(),
            job_name: job.name.clone(),
            started_ms: now,
            finished_ms: now,
            status: "missed".into(),
            summary: "Skipped a firing that came due while the app was closed.".into(),
            output_excerpt: String::new(),
        };
        let _ = Self::append_history(project_root, &rec);
    }

    // ---- background loop (mirrors hooks_service::start_watcher) -----------

    /// Start the scheduler tick loop (idempotent). Runs a one-time reconcile of
    /// firings missed while the app was closed, then polls on a calm interval.
    pub fn start(&self, app: AppHandle) {
        let mut started = self.started.lock();
        if *started {
            return;
        }
        *started = true;
        drop(started);
        thread::spawn(move || {
            // Reconcile missed firings before entering the steady loop.
            {
                let state = app.state::<AppState>();
                state.scheduler.reconcile(&app);
            }
            loop {
                thread::sleep(TICK_INTERVAL);
                let state = app.state::<AppState>();
                state.scheduler.tick(&app);
            }
        });
    }

    /// One-time startup pass: for every job whose `next_due` already passed (the
    /// app was closed when it should have fired), honor `on_missed`. Catchup
    /// jobs keep their stale `next_due` so the first tick runs them once; Skip
    /// jobs are rescheduled forward and recorded as missed. Jobs that never had
    /// a `next_due` (e.g. created while the loop wasn't running) get one.
    fn reconcile(&self, app: &AppHandle) {
        let now = now_ms();
        let projects: Vec<String> = {
            let _g = self.lock.lock();
            match Self::load_file() {
                Ok(f) => f.by_project.keys().cloned().collect(),
                Err(_) => return,
            }
        };
        let mut changed = false;
        for proj in &projects {
            let jobs = match self.list(proj) {
                Ok(j) => j,
                Err(_) => continue,
            };
            for job in jobs {
                if !job.enabled {
                    continue;
                }
                let due = job.next_due_ms;
                match due {
                    None => {
                        // Time-based job missing a schedule → set one.
                        if let Some(next) = compute_next_due(&job.trigger, now) {
                            let _ = self.set_next_due(proj, &job.id, Some(next));
                            changed = true;
                        }
                    }
                    // Missed while closed + Skip policy → record and reschedule.
                    Some(d) if d <= now && job.on_missed == OnMissed::Skip => {
                        self.record_missed(proj, &job);
                        let next = compute_next_due(&job.trigger, now);
                        let _ = self.set_next_due(proj, &job.id, next);
                        changed = true;
                    }
                    // Catchup (d <= now): leave next_due stale; the first tick
                    // runs it once. Future firings (d > now): nothing to do.
                    _ => {}
                }
            }
        }
        if changed {
            let _ = app.emit("scheduler://jobs_changed", serde_json::json!({}));
        }
    }

    fn set_next_due(&self, project_root: &str, id: &str, next: Option<u64>) -> AppResult<()> {
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        if let Some(bucket) = file.by_project.get_mut(project_root) {
            if let Some(j) = bucket.iter_mut().find(|j| j.id == id) {
                j.next_due_ms = next;
            }
        }
        Self::write_file(&file)
    }

    /// One poll: run every enabled, time-based job that has come due across all
    /// projects, respecting the per-job cooldown. Executions are serialized by
    /// the run lock (cap of 1), so a long job just delays the next.
    fn tick(&self, app: &AppHandle) {
        // Global kill-switch: when paused, the loop observes but runs nothing.
        if self.is_paused() {
            return;
        }
        let now = now_ms();
        let projects: Vec<String> = {
            let _g = self.lock.lock();
            match Self::load_file() {
                Ok(f) => f.by_project.keys().cloned().collect(),
                Err(_) => return,
            }
        };
        let mut ran = false;
        for proj in &projects {
            let jobs = match self.list(proj) {
                Ok(j) => j,
                Err(_) => continue,
            };
            for job in jobs {
                if !job.enabled {
                    continue;
                }
                let Some(due) = job.next_due_ms else { continue };
                if due > now {
                    continue;
                }
                if !cooldown_ok(&job, now) || !backoff_ok(&job, now) {
                    continue;
                }
                let rec = self.execute(app, proj, &job);
                let _ = self.mark_ran(proj, &job, &rec);
                ran = true;
            }
        }
        if ran {
            let _ = app.emit("scheduler://jobs_changed", serde_json::json!({}));
        }
    }
}

// ---- action execution (free fns; plan-mode claude) -----------------------

/// Run an action and return ("ok"|"error", combined_output).
fn run_action(project_root: &Path, action: &Action) -> (String, String) {
    match action {
        Action::Skill { name, args } => {
            let prompt = match args {
                Some(a) if !a.trim().is_empty() => format!("/{name} {a}"),
                _ => format!("/{name}"),
            };
            run_claude(project_root, &prompt)
        }
        Action::Prompt { text } => run_claude(project_root, text),
        Action::Pipeline { steps } => run_pipeline(project_root, steps),
    }
}

/// Run pipeline steps in order, each gated by its condition against the previous
/// *executed* step. The overall status is that of the last step that ran. A
/// skipped step is noted in the output but doesn't change the tracked result.
fn run_pipeline(project_root: &Path, steps: &[PipelineStep]) -> (String, String) {
    let mut out = String::new();
    let mut last: Option<(String, String)> = None; // (status, output)
    let mut last_status = "ok".to_string();
    for (i, step) in steps.iter().enumerate() {
        let prev = last.as_ref().map(|(s, o)| (s.as_str(), o.as_str()));
        if !step_should_run(&step.when, prev) {
            out.push_str(&format!("\n--- step {} (skipped: condition not met) ---", i + 1));
            continue;
        }
        let (st, o) = run_action(project_root, &step.action);
        out.push_str(&format!("\n--- step {} [{st}] ---\n{o}", i + 1));
        last_status = st.clone();
        last = Some((st, o));
    }
    (last_status, out)
}

/// Whether a step runs, given its condition and the previous executed step's
/// (status, output). No condition = dependent default (run iff first step or the
/// prior succeeded).
fn step_should_run(when: &Option<StepCondition>, prev: Option<(&str, &str)>) -> bool {
    match when {
        None => match prev {
            None => true,
            Some((status, _)) => status == "ok",
        },
        Some(StepCondition::Contains { text }) => prev
            .map(|(_, o)| o.to_lowercase().contains(&text.to_lowercase()))
            .unwrap_or(false),
        Some(StepCondition::PrevFailed) => prev.map(|(s, _)| s == "error").unwrap_or(false),
        Some(StepCondition::PrevOk) => prev.map(|(s, _)| s == "ok").unwrap_or(false),
    }
}

/// Spawn `claude -p <prompt> --permission-mode plan --output-format text` in the
/// project dir. Plan mode is the suggest-only guarantee: it cannot mutate.
fn run_claude(project_root: &Path, prompt: &str) -> (String, String) {
    let mut cmd = crate::services::claude_cli::command(&[
        "-p",
        prompt,
        "--permission-mode",
        "plan",
        "--output-format",
        "text",
    ]);
    cmd.current_dir(project_root);
    match cmd.output() {
        Ok(o) if o.status.success() => ("ok".into(), String::from_utf8_lossy(&o.stdout).to_string()),
        Ok(o) => (
            "error".into(),
            format!(
                "claude exited {}: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr)
            ),
        ),
        Err(e) => (
            "error".into(),
            format!("failed to spawn `claude`: {e}. Is it on PATH?"),
        ),
    }
}

// ---- pure time math (UTC) ------------------------------------------------

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Whether enough time has passed since the last run to honor `cooldown_ms`.
fn cooldown_ok(job: &Job, now: u64) -> bool {
    match job.last_run_ms {
        Some(last) => job.cooldown_ms == 0 || now.saturating_sub(last) >= job.cooldown_ms,
        None => true,
    }
}

/// Whether a job's post-failure backoff window has elapsed.
fn backoff_ok(job: &Job, now: u64) -> bool {
    match job.backoff_until_ms {
        Some(until) => until <= now,
        None => true,
    }
}

/// Backoff window after `failures` consecutive errors: BASE doubled per failure,
/// capped at MAX. Zero when there are no failures.
fn backoff_delay(failures: u32) -> u64 {
    if failures == 0 {
        return 0;
    }
    let shift = (failures - 1).min(20);
    BACKOFF_BASE_MS
        .saturating_mul(1u64 << shift)
        .min(BACKOFF_MAX_MS)
}

/// Path to the small global scheduler config (kill-switch state).
fn config_path() -> AppResult<PathBuf> {
    Ok(SchedulerService::dir()?.join("scheduler-config.json"))
}

/// Read the persisted global pause flag (default false / not paused).
fn load_paused_flag() -> bool {
    let Ok(path) = config_path() else { return false };
    let Ok(txt) = fs::read_to_string(&path) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&txt)
        .ok()
        .and_then(|v| v.get("paused").and_then(|p| p.as_bool()))
        .unwrap_or(false)
}

/// Persist the global pause flag atomically (temp + rename).
fn save_paused_flag(paused: bool) -> AppResult<()> {
    let path = config_path()?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::json!({ "paused": paused }).to_string())?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Next firing strictly after `after` (epoch ms), or None for non-time triggers.
fn compute_next_due(trigger: &Trigger, after: u64) -> Option<u64> {
    match trigger {
        Trigger::Interval { every_ms } => {
            Some(after.saturating_add((*every_ms).max(1)))
        }
        Trigger::Daily { hour, minute } => Some(next_daily(after, *hour, *minute)),
        Trigger::Weekly {
            weekday,
            hour,
            minute,
        } => Some(next_weekly(after, *weekday, *hour, *minute)),
        Trigger::Event { .. } => None,
    }
}

fn time_of_day_ms(hour: u32, minute: u32) -> u64 {
    let h = (hour % 24) as u64;
    let m = (minute % 60) as u64;
    (h * 60 + m) * 60_000
}

fn next_daily(after: u64, hour: u32, minute: u32) -> u64 {
    let target = time_of_day_ms(hour, minute);
    let day_start = after - (after % DAY_MS);
    let cand = day_start + target;
    if cand > after {
        cand
    } else {
        cand + DAY_MS
    }
}

fn next_weekly(after: u64, weekday: u32, hour: u32, minute: u32) -> u64 {
    let target_tod = time_of_day_ms(hour, minute);
    let day_start = after - (after % DAY_MS);
    // 1970-01-01 (epoch day 0) was a Thursday; with Sunday=0 that's index 4.
    let dow = ((after / DAY_MS) + 4) % 7;
    let want = (weekday % 7) as u64;
    let days_ahead = (want + 7 - dow) % 7;
    let mut cand = day_start + days_ahead * DAY_MS + target_tod;
    if cand <= after {
        cand += 7 * DAY_MS;
    }
    cand
}

fn summarize(output: &str) -> String {
    let line = output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    truncate(line, 200)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_daily_wraps_to_tomorrow_when_past() {
        // after = day start + 10:00 UTC; ask for 09:00 → tomorrow 09:00.
        let day = 20_000 * DAY_MS; // arbitrary whole day
        let after = day + time_of_day_ms(10, 0);
        let next = next_daily(after, 9, 0);
        assert_eq!(next, day + DAY_MS + time_of_day_ms(9, 0));
        // Ask for 11:00 (later today) → today 11:00.
        let next2 = next_daily(after, 11, 0);
        assert_eq!(next2, day + time_of_day_ms(11, 0));
    }

    #[test]
    fn next_weekly_lands_on_requested_weekday() {
        // Epoch day 0 = Thursday (dow 4). Pick a whole day and verify the
        // computed firing falls on the requested weekday at the requested time.
        let base = 21_000 * DAY_MS + time_of_day_ms(8, 0);
        for weekday in 0..7u32 {
            let next = next_weekly(base, weekday, 6, 30);
            let dow = ((next / DAY_MS) + 4) % 7;
            assert_eq!(dow as u32, weekday, "weekday {weekday} mismatch");
            assert_eq!(next % DAY_MS, time_of_day_ms(6, 30));
            assert!(next > base, "must be in the future");
        }
    }

    #[test]
    fn interval_next_is_after_plus_period() {
        let t = compute_next_due(&Trigger::Interval { every_ms: 3_600_000 }, 1_000);
        assert_eq!(t, Some(3_601_000));
        // Event triggers are not time-scheduled.
        assert_eq!(
            compute_next_due(&Trigger::Event { name: "x".into() }, 1_000),
            None
        );
    }

    #[test]
    fn step_conditions_gate_execution() {
        // No condition: first step runs; after that, only if the prior succeeded.
        assert!(step_should_run(&None, None));
        assert!(step_should_run(&None, Some(("ok", "out"))));
        assert!(!step_should_run(&None, Some(("error", "boom"))));

        // Contains is case-insensitive and needs a prior output.
        let c = Some(StepCondition::Contains {
            text: "Anomaly".into(),
        });
        assert!(step_should_run(&c, Some(("ok", "found an anomaly here"))));
        assert!(!step_should_run(&c, Some(("ok", "all clear"))));
        assert!(!step_should_run(&c, None));

        // PrevFailed / PrevOk branch on the prior status.
        assert!(step_should_run(&Some(StepCondition::PrevFailed), Some(("error", ""))));
        assert!(!step_should_run(&Some(StepCondition::PrevFailed), Some(("ok", ""))));
        assert!(step_should_run(&Some(StepCondition::PrevOk), Some(("ok", ""))));
        assert!(!step_should_run(&Some(StepCondition::PrevOk), None));
    }

    #[test]
    fn cooldown_gates_only_within_window() {
        let mut job = Job {
            id: "j".into(),
            name: "j".into(),
            enabled: true,
            trigger: Trigger::Event { name: "e".into() },
            action: Action::Prompt { text: "x".into() },
            on_missed: OnMissed::Catchup,
            cooldown_ms: 1000,
            created_at_ms: 0,
            last_run_ms: None,
            next_due_ms: None,
            consecutive_failures: 0,
            backoff_until_ms: None,
        };
        assert!(cooldown_ok(&job, 5000), "never run → always ok");
        job.last_run_ms = Some(4500);
        assert!(!cooldown_ok(&job, 5000), "within cooldown window → blocked");
        assert!(cooldown_ok(&job, 5600), "past cooldown → ok");
        job.cooldown_ms = 0;
        job.last_run_ms = Some(4999);
        assert!(cooldown_ok(&job, 5000), "no cooldown → always ok");
    }

    #[test]
    fn backoff_grows_then_caps_and_gates() {
        // Doubles per failure from the base, then saturates at the ceiling.
        assert_eq!(backoff_delay(0), 0);
        assert_eq!(backoff_delay(1), BACKOFF_BASE_MS);
        assert_eq!(backoff_delay(2), BACKOFF_BASE_MS * 2);
        assert_eq!(backoff_delay(3), BACKOFF_BASE_MS * 4);
        assert_eq!(backoff_delay(99), BACKOFF_MAX_MS, "caps, never overflows");

        // backoff_ok gates only while the window is in the future.
        let mut job = Job {
            id: "j".into(),
            name: "j".into(),
            enabled: true,
            trigger: Trigger::Interval { every_ms: 1000 },
            action: Action::Prompt { text: "x".into() },
            on_missed: OnMissed::Catchup,
            cooldown_ms: 0,
            created_at_ms: 0,
            last_run_ms: None,
            next_due_ms: None,
            consecutive_failures: 0,
            backoff_until_ms: None,
        };
        assert!(backoff_ok(&job, 1000), "no backoff → ok");
        job.backoff_until_ms = Some(2000);
        assert!(!backoff_ok(&job, 1500), "before window end → blocked");
        assert!(backoff_ok(&job, 2000), "at window end → ok");
    }

    #[test]
    fn jobs_persist_and_reschedule_crash_safe() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-sched-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = SchedulerService::new();
        let proj = "/proj/sched";

        // Fresh: empty, not an error.
        assert!(svc.list(proj).unwrap().is_empty());

        // Create assigns id + a future next_due; atomic write leaves no .tmp.
        let job = Job {
            id: String::new(),
            name: "nightly digest".into(),
            enabled: true,
            trigger: Trigger::Interval { every_ms: 60_000 },
            action: Action::Prompt {
                text: "summarize today".into(),
            },
            on_missed: OnMissed::Catchup,
            cooldown_ms: 0,
            created_at_ms: 0,
            last_run_ms: None,
            next_due_ms: None,
            consecutive_failures: 0,
            backoff_until_ms: None,
        };
        let created = svc.create(proj, job).unwrap();
        assert!(!created.id.is_empty());
        assert!(created.created_at_ms > 0);
        assert!(created.next_due_ms.unwrap() > now_ms() - 1);
        assert!(!SchedulerService::tmp_path().unwrap().exists());
        assert_eq!(svc.list(proj).unwrap().len(), 1);

        // Pause clears nothing but flips enabled; resume reschedules.
        let paused = svc.set_enabled(proj, &created.id, false).unwrap();
        assert!(!paused.enabled);
        let resumed = svc.set_enabled(proj, &created.id, true).unwrap();
        assert!(resumed.enabled);
        assert!(resumed.next_due_ms.is_some());

        // Update changes the action and recomputes next_due.
        let mut edited = resumed.clone();
        edited.action = Action::Skill {
            name: "reflect".into(),
            args: None,
        };
        let updated = svc.update(proj, edited).unwrap();
        assert!(matches!(updated.action, Action::Skill { .. }));
        assert_eq!(updated.created_at_ms, created.created_at_ms);

        // History round-trips through the ledger.
        SchedulerService::append_history(
            proj,
            &RunRecord {
                job_id: created.id.clone(),
                job_name: "nightly digest".into(),
                started_ms: 1,
                finished_ms: 2,
                status: "ok".into(),
                summary: "did a thing".into(),
                output_excerpt: "output".into(),
            },
        )
        .unwrap();
        let hist = svc.history(proj, None).unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].status, "ok");

        // Delete removes the job (and the now-empty bucket).
        svc.delete(proj, &created.id).unwrap();
        assert!(svc.list(proj).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }
}
