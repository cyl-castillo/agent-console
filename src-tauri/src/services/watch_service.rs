//! Goal watch (experimental): the human states what they are trying to get
//! done in a project, and the console follows the runs the ledger witnesses
//! and says so when one of them needs a decision.
//!
//! Phase 1 knows exactly one condition, and it is one Testigo can already
//! prove: a `check_run` line whose status is `failed` (a recognized test or
//! check runner that exited non-zero inside a turn). Everything else here is
//! plumbing around that fact:
//!
//! - **Read-only against the ledger.** This module calls
//!   `TestigoService::head` and `::list` and nothing else. It never writes a
//!   ledger line and `testigo_service.rs` does not know it exists: a bug here
//!   cannot break, slow or reorder evidence.
//! - **A cursor, not a subscription.** Each goal keeps the last ledger `seq`
//!   it evaluated. A tick compares it with the ledger head and evaluates what
//!   is new. What happened while the app was closed is recovered by the same
//!   code path on the first tick after launch: the lines are on disk, the
//!   cursor is behind them.
//! - **One alert per failing run.** The dedupe key is the turn. A second
//!   failed check inside the same turn joins the existing alert (its `seq` is
//!   added to the evidence) and knocks nobody. The `notified` flag is written
//!   to disk BEFORE the notification is sent, so a restart never repeats one.
//! - **Deterministic.** No model. Questions are matched by a small grammar
//!   and answered from the alert's own ledger lines; an answer names what is
//!   missing instead of guessing.
//! - **Demo source.** A goal can follow a scripted timeline instead of the
//!   ledger. Its events are a pure function of the goal's start time and the
//!   clock, so they keep "happening" with the window in the background or
//!   the app closed, and they never touch a real ledger.
//!
//! State lives in one crash-safe `watch.json` (temp-write + `.bak` + rename),
//! keyed by project, like `scheduler.json`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};
use crate::services::testigo_service::{ProofEvent, TestigoService};
use crate::state::AppState;

/// How often the loop compares cursors with ledger heads. The comparison is
/// a cached lookup; the ledger file is only read when the head moved.
const TICK_INTERVAL: Duration = Duration::from_secs(3);
const TITLE_MAX: usize = 200;
const QUESTION_MAX: usize = 500;
/// Oldest alerts and exchanges fall off past these; the ledger keeps the
/// evidence either way.
const ALERTS_MAX: usize = 200;
const THREAD_MAX: usize = 50;

pub const CONDITION_CHECK_FAILED: &str = "check_failed";
pub const SOURCE_LEDGER: &str = "ledger";
pub const SOURCE_DEMO: &str = "demo";

/// What the human is told, with the ledger lines that justify it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Alert {
    pub id: String,
    /// Which rule fired. Only `check_failed` exists.
    pub condition: String,
    /// Dedupe key: the turn id, or `seq:<n>` for a check outside any turn.
    pub key: String,
    /// Timestamp of the first triggering line, and of the latest one.
    pub ts: i64,
    pub last_ts: i64,
    pub turn_id: Option<String>,
    pub term_id: Option<String>,
    pub case_id: String,
    /// The first failing command, as the ledger recorded it (redacted there).
    pub command: String,
    pub check_kind: String,
    pub exit_code: Option<i64>,
    /// The snapshot the turn started from, when the ledger has one.
    pub revision: Option<String>,
    /// `seq` of every `check_run` line that fed this alert, in order.
    pub evidence_seqs: Vec<u64>,
    /// new | seen | dismissed
    pub state: String,
    /// Written before the notification goes out; never reset.
    pub notified: bool,
    /// Found on the catch-up pass after launch: it happened while the app
    /// was closed.
    pub recovered: bool,
}

/// One question and its answer, kept so the thread survives a restart.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Exchange {
    pub ts: i64,
    pub question: String,
    pub answer: String,
    /// run_summary | status | ambiguous | no_alerts | unknown
    pub intent: String,
    /// The alert the answer is about, when it resolved to one.
    pub alert_id: Option<String>,
    /// Ledger lines the answer was built from.
    pub evidence_seqs: Vec<u64>,
    /// What the ledger did not have. Shown, never papered over.
    pub missing: Vec<String>,
    /// Alert ids to choose from when "that run" could be more than one.
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Goal {
    pub id: String,
    /// The outcome the human wants, in their words.
    pub title: String,
    /// ledger | demo
    pub source: String,
    /// active | paused | stopped
    pub status: String,
    pub conditions: Vec<String>,
    pub created_ms: i64,
    /// Last ledger `seq` evaluated. Starts at the head the ledger had when
    /// the watch began: earlier runs are not this goal's business.
    pub cursor_seq: Option<u64>,
    pub alerts: Vec<Alert>,
    pub thread: Vec<Exchange>,
}

/// What the watch can see of a project's ledger right now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    /// False when witnessing is switched off for the project: nothing new
    /// will reach the ledger, so nothing can be followed.
    pub witness: bool,
    pub head_seq: Option<u64>,
}

/// An alert the tick just created.
#[derive(Debug, Clone)]
pub struct Fired {
    pub project_root: String,
    pub goal_id: String,
    pub goal_title: String,
    pub alert: Alert,
}

#[derive(Debug, Default)]
pub struct TickOutcome {
    pub fired: Vec<Fired>,
    /// Projects whose goals changed (alerts, cursors).
    pub changed: Vec<String>,
}

/// Version of this store's file on disk (its top-level `schemaVersion`).
/// Raise it only together with a migration from every earlier version.
const STORE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct WatchFile {
    by_project: HashMap<String, Vec<Goal>>,
}

pub struct WatchService {
    /// Guards the file (read-modify-write).
    lock: Mutex<()>,
    started: Mutex<bool>,
}

impl Default for WatchService {
    fn default() -> Self {
        Self::new()
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl WatchService {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            started: Mutex::new(false),
        }
    }

    // ---- file (crash-safe, mirrors scheduler_service) ----------------------

    fn path(ext: &str) -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir.join(format!("watch.{ext}")))
    }

    fn load_file() -> AppResult<WatchFile> {
        crate::services::persistence::load_json_store(
            &Self::path("json")?,
            &Self::path("json.bak")?,
            "watch.json",
            Some(STORE_SCHEMA_VERSION),
        )
    }

    fn write_file(file: &WatchFile) -> AppResult<()> {
        crate::services::persistence::save_json_store(
            &Self::path("json")?,
            &Self::path("json.bak")?,
            &Self::path("json.tmp")?,
            "watch.json",
            Some(STORE_SCHEMA_VERSION),
            file,
        )
    }

    /// Read-modify-write one goal under the lock.
    fn with_goal<T>(
        &self,
        project_root: &str,
        id: &str,
        f: impl FnOnce(&mut Goal) -> AppResult<T>,
    ) -> AppResult<T> {
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        let goal = file
            .by_project
            .get_mut(project_root)
            .and_then(|gs| gs.iter_mut().find(|g| g.id == id))
            .ok_or_else(|| AppError::NotFound(format!("goal {id}")))?;
        let out = f(goal)?;
        Self::write_file(&file)?;
        Ok(out)
    }

    // ---- public API --------------------------------------------------------

    /// Goals of a project, newest first.
    pub fn list(&self, project_root: &str) -> AppResult<Vec<Goal>> {
        let _g = self.lock.lock();
        let mut goals = Self::load_file()?
            .by_project
            .remove(project_root)
            .unwrap_or_default();
        goals.sort_by_key(|g| std::cmp::Reverse(g.created_ms));
        Ok(goals)
    }

    /// Begin following a goal. A ledger goal starts at the ledger's current
    /// head; a demo goal starts its scripted timeline at `now`.
    pub fn start(
        &self,
        testigo: &TestigoService,
        project_root: &str,
        title: &str,
        demo: bool,
        now: i64,
    ) -> AppResult<Goal> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::InvalidArgument("a goal needs a title".into()));
        }
        let title: String = title.chars().take(TITLE_MAX).collect();
        let cursor_seq = if demo {
            None
        } else {
            testigo.head(project_root)?.map(|(seq, _)| seq)
        };
        let goal = Goal {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            source: if demo { SOURCE_DEMO } else { SOURCE_LEDGER }.into(),
            status: "active".into(),
            conditions: vec![CONDITION_CHECK_FAILED.into()],
            created_ms: now,
            cursor_seq,
            alerts: Vec::new(),
            thread: Vec::new(),
        };
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        file.by_project
            .entry(project_root.to_string())
            .or_default()
            .push(goal.clone());
        Self::write_file(&file)?;
        Ok(goal)
    }

    /// Pause, resume or stop. Stopped is final: a stopped goal keeps its
    /// alerts and thread for reading and is never evaluated again. A paused
    /// goal keeps its cursor, so resuming evaluates what it skipped.
    pub fn set_status(&self, project_root: &str, id: &str, status: &str) -> AppResult<Goal> {
        if !["active", "paused", "stopped"].contains(&status) {
            return Err(AppError::InvalidArgument(format!("status {status}")));
        }
        self.with_goal(project_root, id, |g| {
            if g.status == "stopped" && status != "stopped" {
                return Err(AppError::InvalidArgument(
                    "a stopped goal cannot be resumed; start a new one".into(),
                ));
            }
            g.status = status.into();
            Ok(g.clone())
        })
    }

    pub fn delete(&self, project_root: &str, id: &str) -> AppResult<()> {
        let _g = self.lock.lock();
        let mut file = Self::load_file()?;
        if let Some(goals) = file.by_project.get_mut(project_root) {
            goals.retain(|g| g.id != id);
            if goals.is_empty() {
                file.by_project.remove(project_root);
            }
        }
        Self::write_file(&file)
    }

    pub fn set_alert_state(
        &self,
        project_root: &str,
        goal_id: &str,
        alert_id: &str,
        state: &str,
    ) -> AppResult<Goal> {
        if !["new", "seen", "dismissed"].contains(&state) {
            return Err(AppError::InvalidArgument(format!("alert state {state}")));
        }
        self.with_goal(project_root, goal_id, |g| {
            let alert = g
                .alerts
                .iter_mut()
                .find(|a| a.id == alert_id)
                .ok_or_else(|| AppError::NotFound(format!("alert {alert_id}")))?;
            alert.state = state.into();
            Ok(g.clone())
        })
    }

    pub fn source_status(&self, testigo: &TestigoService, project_root: &str) -> SourceStatus {
        SourceStatus {
            witness: testigo.settings(project_root).witness,
            head_seq: testigo
                .head(project_root)
                .ok()
                .flatten()
                .map(|(seq, _)| seq),
        }
    }

    /// Every event the goal's source has produced so far.
    fn source_events(
        testigo: &TestigoService,
        project_root: &str,
        goal: &Goal,
        now: i64,
    ) -> AppResult<Vec<ProofEvent>> {
        if goal.source == SOURCE_DEMO {
            Ok(demo_events(goal.created_ms, now))
        } else {
            testigo.list(project_root, None, None)
        }
    }

    /// The ledger lines behind an alert: its whole turn when it has one (the
    /// prompt, the tool calls, the checks, the close), else the check lines.
    pub fn evidence(
        &self,
        testigo: &TestigoService,
        project_root: &str,
        goal_id: &str,
        alert_id: &str,
        now: i64,
    ) -> AppResult<Vec<ProofEvent>> {
        let goal = self.find(project_root, goal_id)?;
        let alert = goal
            .alerts
            .iter()
            .find(|a| a.id == alert_id)
            .ok_or_else(|| AppError::NotFound(format!("alert {alert_id}")))?;
        let events = Self::source_events(testigo, project_root, &goal, now)?;
        Ok(alert_events(alert, &events))
    }

    fn find(&self, project_root: &str, goal_id: &str) -> AppResult<Goal> {
        self.list(project_root)?
            .into_iter()
            .find(|g| g.id == goal_id)
            .ok_or_else(|| AppError::NotFound(format!("goal {goal_id}")))
    }

    /// Answer a question about the goal from its ledger lines, and keep the
    /// exchange. `focus` is the alert the human is looking at: "that run".
    pub fn ask(
        &self,
        testigo: &TestigoService,
        project_root: &str,
        goal_id: &str,
        question: &str,
        focus: Option<&str>,
        now: i64,
    ) -> AppResult<Exchange> {
        let question: String = question.trim().chars().take(QUESTION_MAX).collect();
        if question.is_empty() {
            return Err(AppError::InvalidArgument("empty question".into()));
        }
        let goal = self.find(project_root, goal_id)?;
        // A failed read is said, never turned into "nothing happened".
        let events = Self::source_events(testigo, project_root, &goal, now);
        let mut ex = match &events {
            Ok(events) => answer(&goal, events, &question, focus),
            Err(e) => {
                let es = is_spanish(&normalize(&question));
                Exchange {
                    intent: "unknown".into(),
                    answer: if es {
                        format!("No pude leer el ledger de este proyecto ({e}), así que no puedo responder con evidencia.")
                    } else {
                        format!("I could not read this project's ledger ({e}), so I cannot answer from evidence.")
                    },
                    missing: vec!["ledger".into()],
                    ..Exchange::default()
                }
            }
        };
        ex.ts = now;
        ex.question = question;
        let stored = ex.clone();
        self.with_goal(project_root, goal_id, move |g| {
            g.thread.push(stored);
            let extra = g.thread.len().saturating_sub(THREAD_MAX);
            g.thread.drain(..extra);
            Ok(())
        })?;
        Ok(ex)
    }

    // ---- evaluation --------------------------------------------------------

    /// One pass over every active goal. `catch_up` marks the first pass after
    /// launch: what it finds happened while the app was closed.
    ///
    /// Alerts are saved with `notified = true` before this returns, so the
    /// caller may notify without any further bookkeeping and a crash between
    /// the two loses at most a knock, never repeats one.
    pub fn tick_core(&self, testigo: &TestigoService, now: i64, catch_up: bool) -> TickOutcome {
        let mut out = TickOutcome::default();
        let _g = self.lock.lock();
        let Ok(mut file) = Self::load_file() else {
            return out;
        };
        for (root, goals) in file.by_project.iter_mut() {
            let mut changed = false;
            for goal in goals.iter_mut().filter(|g| g.status == "active") {
                if goal.source == SOURCE_LEDGER {
                    let head = match testigo.head(root) {
                        Ok(h) => h.map(|(seq, _)| seq),
                        Err(_) => continue,
                    };
                    if head == goal.cursor_seq || head.is_none() {
                        continue;
                    }
                    // The ledger is never trimmed; a head behind the cursor
                    // means the file was replaced. Follow the new one from
                    // here instead of waiting for it to outgrow the old seq.
                    if head < goal.cursor_seq {
                        goal.cursor_seq = head;
                        changed = true;
                        continue;
                    }
                }
                let Ok(events) = Self::source_events(testigo, root, goal, now) else {
                    continue;
                };
                let before = (goal.cursor_seq, goal.alerts.clone());
                for alert in evaluate(goal, &events, catch_up) {
                    out.fired.push(Fired {
                        project_root: root.clone(),
                        goal_id: goal.id.clone(),
                        goal_title: goal.title.clone(),
                        alert,
                    });
                }
                changed |= before != (goal.cursor_seq, goal.alerts.clone());
            }
            if changed {
                out.changed.push(root.clone());
            }
        }
        if !out.changed.is_empty() && Self::write_file(&file).is_err() {
            // Nothing was saved: firing now would fire again next tick.
            return TickOutcome::default();
        }
        out
    }

    /// Start the loop (idempotent): a catch-up pass, then a calm poll.
    pub fn start_loop(&self, app: AppHandle) {
        let mut started = self.started.lock();
        if *started {
            return;
        }
        *started = true;
        drop(started);
        thread::spawn(move || {
            let mut catch_up = true;
            loop {
                let state = app.state::<AppState>();
                let out = state.watch.tick_core(&state.testigo, now_ms(), catch_up);
                announce(&app, &out);
                catch_up = false;
                thread::sleep(TICK_INTERVAL);
            }
        });
    }
}

/// Tell the views, and knock when nobody is looking at the window.
fn announce(app: &AppHandle, out: &TickOutcome) {
    for root in &out.changed {
        let _ = app.emit("watch://changed", json!({ "projectRoot": root }));
    }
    if out.fired.is_empty() {
        return;
    }
    for f in &out.fired {
        let _ = app.emit(
            "watch://alert",
            json!({
                "projectRoot": f.project_root,
                "goalId": f.goal_id,
                "alertId": f.alert.id,
                "command": f.alert.command,
                "recovered": f.alert.recovered,
            }),
        );
    }
    // Looking at the app already: the panel and its toast are the notice.
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if focused {
        return;
    }
    let (title, body) = notification_text(&out.fired);
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("watch: notification failed: {e}");
    }
}

/// One notification per pass: a single failure is named, several (a catch-up
/// after hours closed) are counted.
pub fn notification_text(fired: &[Fired]) -> (String, String) {
    match fired {
        [one] => (
            "A check failed".to_string(),
            format!(
                "{} · {}",
                truncate(&one.alert.command, 80),
                truncate(&one.goal_title, 80)
            ),
        ),
        many => (
            format!("{} runs failed a check", many.len()),
            truncate(&many[0].goal_title, 120),
        ),
    }
}

/// Fold the events past the goal's cursor into its alerts. Returns the alerts
/// created by this call (already marked notified). Pure: no clock, no disk.
pub fn evaluate(goal: &mut Goal, events: &[ProofEvent], catch_up: bool) -> Vec<Alert> {
    let mut created: Vec<String> = Vec::new();
    let cursor = goal.cursor_seq;
    for e in events.iter().filter(|e| cursor.is_none_or(|c| e.seq > c)) {
        goal.cursor_seq = Some(goal.cursor_seq.map_or(e.seq, |c| c.max(e.seq)));
        if e.kind != "check_run" || str_at(&e.payload, "status") != Some("failed") {
            continue;
        }
        let key = e
            .turn_id
            .clone()
            .unwrap_or_else(|| format!("seq:{}", e.seq));
        if let Some(alert) = goal.alerts.iter_mut().find(|a| a.key == key) {
            alert.evidence_seqs.push(e.seq);
            alert.last_ts = e.ts;
            continue;
        }
        let id = uuid::Uuid::new_v4().to_string();
        created.push(id.clone());
        goal.alerts.push(Alert {
            id,
            condition: CONDITION_CHECK_FAILED.into(),
            key,
            ts: e.ts,
            last_ts: e.ts,
            turn_id: e.turn_id.clone(),
            term_id: e.term_id.clone(),
            case_id: e.case_id.clone(),
            command: str_at(&e.payload, "command").unwrap_or("").to_string(),
            check_kind: str_at(&e.payload, "kind").unwrap_or("test").to_string(),
            exit_code: e.payload.get("exitCode").and_then(Value::as_i64),
            revision: e
                .turn_id
                .as_deref()
                .and_then(|t| turn_revision(events, t, e.seq)),
            evidence_seqs: vec![e.seq],
            state: "new".into(),
            notified: true,
            recovered: catch_up,
        });
    }
    let extra = goal.alerts.len().saturating_sub(ALERTS_MAX);
    goal.alerts.drain(..extra);
    goal.alerts
        .iter()
        .filter(|a| created.contains(&a.id))
        .cloned()
        .collect()
}

/// The snapshot a turn started from: its first `snapshot` line at or before
/// `upto`.
fn turn_revision(events: &[ProofEvent], turn_id: &str, upto: u64) -> Option<String> {
    events
        .iter()
        .filter(|e| e.kind == "snapshot" && e.seq <= upto)
        .find(|e| e.turn_id.as_deref() == Some(turn_id))
        .and_then(|e| str_at(&e.payload, "commitSha"))
        .map(String::from)
}

fn alert_events(alert: &Alert, events: &[ProofEvent]) -> Vec<ProofEvent> {
    events
        .iter()
        .filter(|e| match &alert.turn_id {
            Some(t) => e.turn_id.as_deref() == Some(t),
            None => alert.evidence_seqs.contains(&e.seq),
        })
        .cloned()
        .collect()
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

// ---- questions -------------------------------------------------------------

/// Lowercase, no diacritics, no punctuation, single spaces.
fn normalize(s: &str) -> String {
    let mapped: String = s
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_word(norm: &str, words: &[&str]) -> bool {
    norm.split(' ').any(|w| words.contains(&w))
}

fn is_spanish(norm: &str) -> bool {
    has_word(
        norm,
        &[
            "que",
            "paso",
            "fallo",
            "ejecucion",
            "esa",
            "ese",
            "como",
            "por",
            "cual",
            "ultima",
            "ultimo",
            "estado",
            "estas",
            "va",
            "ocurrio",
            "aviso",
            "corrida",
        ],
    )
}

/// Match the question against the few things phase 1 can answer.
pub fn answer(goal: &Goal, events: &[ProofEvent], question: &str, focus: Option<&str>) -> Exchange {
    let norm = normalize(question);
    let es = is_spanish(&norm);
    let asks_run = [
        "que paso",
        "que ocurrio",
        "que fallo",
        "por que fallo",
        "what happened",
        "what failed",
        "why did it fail",
        "what went wrong",
    ]
    .iter()
    .any(|p| norm.contains(p));
    let asks_status = [
        "como va",
        "que estas siguiendo",
        "que seguis",
        "estado",
        "how is it going",
        "what are you watching",
        "status",
    ]
    .iter()
    .any(|p| norm.contains(p));

    if asks_run {
        return answer_run(goal, events, &norm, focus, es);
    }
    if asks_status {
        return answer_status(goal, es);
    }
    Exchange {
        intent: "unknown".into(),
        answer: if es {
            "Todavía no sé responder eso. En esta fase puedo decirte qué pasó en una ejecución que falló («¿qué pasó en esa ejecución?») y cómo va el seguimiento («¿cómo va?»).".into()
        } else {
            "I cannot answer that yet. In this phase I can tell you what happened in a run that failed (\"what happened in that run?\") and how the watch is going (\"status\").".into()
        },
        ..Exchange::default()
    }
}

fn answer_status(goal: &Goal, es: bool) -> Exchange {
    let open = goal
        .alerts
        .iter()
        .filter(|a| a.state != "dismissed")
        .count();
    let upto = match goal.cursor_seq {
        Some(s) => format!("seq {s}"),
        None if es => "ningún evento todavía".into(),
        None => "no events yet".into(),
    };
    let answer = if es {
        format!(
            "Objetivo: «{}». Seguimiento {}. Condición: una ejecución termina con un check fallido. Avisos abiertos: {open}. Evalué hasta {upto}.",
            goal.title,
            match goal.status.as_str() {
                "active" => "activo",
                "paused" => "en pausa",
                _ => "detenido",
            }
        )
    } else {
        format!(
            "Goal: \"{}\". The watch is {}. Condition: a run ends with a failed check. Open alerts: {open}. Evaluated up to {upto}.",
            goal.title, goal.status
        )
    };
    Exchange {
        intent: "status".into(),
        answer,
        ..Exchange::default()
    }
}

fn answer_run(
    goal: &Goal,
    events: &[ProofEvent],
    norm: &str,
    focus: Option<&str>,
    es: bool,
) -> Exchange {
    let open: Vec<&Alert> = goal
        .alerts
        .iter()
        .filter(|a| a.state != "dismissed")
        .collect();
    if goal.alerts.is_empty() {
        let upto = goal
            .cursor_seq
            .map(|s| format!("seq {s}"))
            .unwrap_or_else(|| if es { "sin eventos" } else { "no events" }.into());
        return Exchange {
            intent: "no_alerts".into(),
            answer: if es {
                format!("No hay ningún aviso en este seguimiento: no vi ningún check fallido en lo que evalué (hasta {upto}). Eso no dice nada de ejecuciones que el ledger no presenció.")
            } else {
                format!("There is no alert in this watch: I saw no failed check in what I evaluated (up to {upto}). That says nothing about runs the ledger did not witness.")
            },
            ..Exchange::default()
        };
    }
    let wants_last = has_word(norm, &["ultima", "ultimo", "last", "latest"]);
    let focused = focus.and_then(|id| goal.alerts.iter().find(|a| a.id == id));
    let alert = match (wants_last, focused) {
        (true, _) => goal.alerts.last(),
        (false, Some(a)) => Some(a),
        (false, None) if open.len() == 1 => Some(open[0]),
        (false, None) if open.is_empty() => goal.alerts.last(),
        (false, None) => None,
    };
    let Some(alert) = alert else {
        return Exchange {
            intent: "ambiguous".into(),
            answer: if es {
                format!("Hay {} ejecuciones con aviso. ¿Cuál?", open.len())
            } else {
                format!("There are {} runs with an alert. Which one?", open.len())
            },
            candidates: open.iter().map(|a| a.id.clone()).collect(),
            ..Exchange::default()
        };
    };
    let turn = alert_events(alert, events);
    let (answer, missing) = describe_run(alert, &turn, es);
    Exchange {
        intent: "run_summary".into(),
        answer,
        alert_id: Some(alert.id.clone()),
        evidence_seqs: turn.iter().map(|e| e.seq).collect(),
        missing,
        ..Exchange::default()
    }
}

/// What the ledger says about one run, and what it does not.
fn describe_run(alert: &Alert, turn: &[ProofEvent], es: bool) -> (String, Vec<String>) {
    let mut parts: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let miss = |m: &mut Vec<String>, en: &str, sp: &str| {
        let text = if es { sp } else { en }.to_string();
        if !m.contains(&text) {
            m.push(text);
        }
    };

    if turn.is_empty() {
        miss(
            &mut missing,
            "the ledger lines of this alert (they are no longer readable)",
            "las líneas del ledger de este aviso (ya no se pueden leer)",
        );
    }
    match turn
        .iter()
        .find(|e| e.kind == "prompt")
        .and_then(|e| str_at(&e.payload, "prompt"))
    {
        Some(p) => parts.push(if es {
            format!("El pedido fue: «{}».", truncate(p, 160))
        } else {
            format!("The prompt was: \"{}\".", truncate(p, 160))
        }),
        None => miss(
            &mut missing,
            "the prompt that opened the run",
            "el pedido que abrió la ejecución",
        ),
    }
    let tools = turn.iter().filter(|e| e.kind == "tool_result").count();
    if tools > 0 {
        parts.push(if es {
            format!("Registró {tools} llamadas a herramientas.")
        } else {
            format!("It recorded {tools} tool calls.")
        });
    }
    let checks: Vec<&ProofEvent> = turn.iter().filter(|e| e.kind == "check_run").collect();
    let failed: Vec<&&ProofEvent> = checks
        .iter()
        .filter(|e| str_at(&e.payload, "status") == Some("failed"))
        .collect();
    let passed = checks
        .iter()
        .filter(|e| str_at(&e.payload, "status") == Some("passed"))
        .count();
    for e in &failed {
        let cmd = truncate(str_at(&e.payload, "command").unwrap_or("?"), 120);
        let code = e.payload.get("exitCode").and_then(Value::as_i64);
        parts.push(match (es, code) {
            (true, Some(c)) => format!("Falló `{cmd}` con código de salida {c} (seq {}).", e.seq),
            (true, None) => format!("Falló `{cmd}` (seq {}).", e.seq),
            (false, Some(c)) => format!("`{cmd}` failed with exit code {c} (seq {}).", e.seq),
            (false, None) => format!("`{cmd}` failed (seq {}).", e.seq),
        });
        if code.is_none() {
            miss(
                &mut missing,
                "the exit code (the engine did not report it)",
                "el código de salida (el motor no lo informó)",
            );
        }
    }
    if passed > 0 {
        parts.push(if es {
            format!("Otros {passed} checks pasaron en la misma ejecución.")
        } else {
            format!("{passed} other checks passed in the same run.")
        });
    }
    match &alert.revision {
        Some(r) => parts.push(if es {
            format!("Partió de la revisión {}.", short_sha(r))
        } else {
            format!("It started from revision {}.", short_sha(r))
        }),
        None => miss(
            &mut missing,
            "the revision the run started from (no snapshot in the ledger)",
            "la revisión de la que partió (no hay snapshot en el ledger)",
        ),
    }
    match turn.iter().find(|e| e.kind == "turn_end") {
        Some(end) => {
            let files: Vec<String> = end
                .payload
                .get("filesChanged")
                .and_then(Value::as_array)
                .map(|fs| {
                    fs.iter()
                        .filter_map(|f| str_at(f, "path"))
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            if files.is_empty() {
                miss(
                    &mut missing,
                    "the files the run changed (no diff recorded)",
                    "los archivos que cambió (no hay diff registrado)",
                );
            } else {
                let shown = files.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
                let more = files.len().saturating_sub(5);
                let tail = if more > 0 {
                    format!(" (+{more})")
                } else {
                    String::new()
                };
                parts.push(if es {
                    format!("Cambió: {shown}{tail}.")
                } else {
                    format!("It changed: {shown}{tail}.")
                });
            }
        }
        None => miss(
            &mut missing,
            "the end of the run (it is still open, so there is no diff yet)",
            "el cierre de la ejecución (sigue abierta, todavía no hay diff)",
        ),
    }
    if let Some(v) = turn.iter().find(|e| e.kind == "verification") {
        if let Some(status) = str_at(&v.payload, "status") {
            let reason = str_at(&v.payload, "reason").unwrap_or("");
            parts.push(if es {
                format!("Verificación de la consola: {status}. {reason}")
            } else {
                format!("Console verification: {status}. {reason}")
            });
        }
    }
    if !missing.is_empty() {
        parts.push(if es {
            format!("Me falta: {}.", missing.join("; "))
        } else {
            format!("Missing: {}.", missing.join("; "))
        });
    }
    (parts.join(" ").trim().to_string(), missing)
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

// ---- demo source -----------------------------------------------------------

/// Case id every demo event carries; the UI labels the goal as a demo.
pub const DEMO_CASE: &str = "demo:pr-142";
/// When the demo's failing check lands, in ms after the goal started.
pub const DEMO_FAILURE_AT_MS: i64 = 30_000;

/// The scripted timeline of a demo goal, up to `now`: one turn that passes
/// its tests, then one that fails them twice. Timestamps are offsets from
/// the goal's start, so the events keep arriving whether or not the window
/// is in front — and are simply already there after a restart.
///
/// These are ProofEvent-shaped but they are not evidence: they are never
/// written to a ledger and their hashes say so.
pub fn demo_events(origin_ms: i64, now: i64) -> Vec<ProofEvent> {
    let pre1 = "3f9a1c2e7b604d15a8c0e4f2b79d6a31c5e08f47";
    let post1 = "8b2e6d40c1f75a93e0d4b68a2c7f1953d6e4a0b8";
    let post2 = "c47d0a9f3e6b21850f7c4d2a9e6b0138f5a7c2d6";
    let t1 = "demo-turn-1";
    let t2 = "demo-turn-2";
    let script: Vec<(i64, &str, &str, &str, Value)> = vec![
        (
            4_000,
            "prompt",
            t1,
            "human",
            json!({ "prompt": "Round order totals per line before adding tax", "cwd": "/demo/shop" }),
        ),
        (
            5_000,
            "snapshot",
            t1,
            "system",
            json!({ "commitSha": pre1 }),
        ),
        (
            9_000,
            "tool_result",
            t1,
            "agent",
            json!({ "tool": "apply_patch", "toolCanonical": "edit", "excerpt": "M src/totals.ts" }),
        ),
        (
            13_000,
            "check_run",
            t1,
            "agent",
            json!({ "command": "npm test -- totals", "kind": "test", "status": "passed", "exitCode": 0 }),
        ),
        (
            15_000,
            "turn_end",
            t1,
            "agent",
            json!({
                "preSha": pre1, "postSha": post1,
                "filesChanged": [{ "status": "M", "path": "src/totals.ts" }],
                "summary": "Totals are rounded per line. Tests pass.",
            }),
        ),
        (
            20_000,
            "prompt",
            t2,
            "human",
            json!({ "prompt": "Apply the discount before tax instead of after", "cwd": "/demo/shop" }),
        ),
        (
            21_000,
            "snapshot",
            t2,
            "system",
            json!({ "commitSha": post1 }),
        ),
        (
            26_000,
            "tool_result",
            t2,
            "agent",
            json!({ "tool": "apply_patch", "toolCanonical": "edit", "excerpt": "M src/totals.ts" }),
        ),
        (
            DEMO_FAILURE_AT_MS,
            "check_run",
            t2,
            "agent",
            json!({ "command": "npm test -- totals", "kind": "test", "status": "failed", "exitCode": 1 }),
        ),
        (
            34_000,
            "check_run",
            t2,
            "agent",
            json!({ "command": "npm test", "kind": "test", "status": "failed", "exitCode": 1 }),
        ),
        (
            38_000,
            "turn_end",
            t2,
            "agent",
            json!({
                "preSha": post1, "postSha": post2,
                "filesChanged": [
                    { "status": "M", "path": "src/totals.ts" },
                    { "status": "M", "path": "src/totals.test.ts" },
                ],
                "summary": "Moved the discount before tax. 2 tests in totals.test.ts still fail.",
            }),
        ),
    ];
    script
        .into_iter()
        .enumerate()
        .map(|(i, (offset, kind, turn, actor, payload))| ProofEvent {
            seq: i as u64,
            ts: origin_ms + offset,
            case_id: DEMO_CASE.into(),
            turn_id: Some(turn.into()),
            kind: kind.into(),
            term_id: Some("demo-codex".into()),
            session_id: None,
            actor: actor.into(),
            payload,
            prev_hash: "demo".into(),
            hash: format!("demo-{i}"),
        })
        .filter(|e| e.ts <= now)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, kind: &str, turn: Option<&str>, payload: Value) -> ProofEvent {
        ProofEvent {
            seq,
            ts: 1_000 + seq as i64,
            case_id: "term:t1".into(),
            turn_id: turn.map(String::from),
            kind: kind.into(),
            term_id: Some("t1".into()),
            session_id: None,
            actor: "agent".into(),
            payload,
            prev_hash: String::new(),
            hash: String::new(),
        }
    }

    fn failed(seq: u64, turn: Option<&str>, cmd: &str) -> ProofEvent {
        ev(
            seq,
            "check_run",
            turn,
            json!({ "command": cmd, "kind": "test", "status": "failed", "exitCode": 1 }),
        )
    }

    fn goal(cursor: Option<u64>) -> Goal {
        Goal {
            id: "g".into(),
            title: "Close the PR today".into(),
            source: SOURCE_LEDGER.into(),
            status: "active".into(),
            conditions: vec![CONDITION_CHECK_FAILED.into()],
            cursor_seq: cursor,
            ..Goal::default()
        }
    }

    #[test]
    fn one_alert_per_failing_run_and_repeats_join_it() {
        let events = vec![
            ev(0, "prompt", Some("a"), json!({ "prompt": "fix totals" })),
            ev(
                1,
                "snapshot",
                Some("a"),
                json!({ "commitSha": "abc12345ff" }),
            ),
            ev(
                2,
                "check_run",
                Some("a"),
                json!({ "command": "npm test", "status": "passed", "exitCode": 0 }),
            ),
            failed(3, Some("a"), "cargo test"),
            failed(4, Some("a"), "cargo test --workspace"),
            failed(5, Some("b"), "npm test"),
            failed(6, None, "make test"),
        ];
        let mut g = goal(None);
        let fired = evaluate(&mut g, &events, false);
        assert_eq!(fired.len(), 3, "turn a, turn b, the turnless check");
        assert_eq!(g.alerts[0].evidence_seqs, vec![3, 4]);
        assert_eq!(g.alerts[0].command, "cargo test");
        assert_eq!(g.alerts[0].revision.as_deref(), Some("abc12345ff"));
        assert_eq!(g.alerts[1].revision, None, "turn b has no snapshot");
        assert_eq!(g.alerts[2].key, "seq:6");
        assert!(fired.iter().all(|a| a.notified && !a.recovered));
        assert_eq!(g.cursor_seq, Some(6));

        // The same events again: the cursor is past them. Nothing fires.
        assert!(evaluate(&mut g, &events, false).is_empty());
        assert_eq!(g.alerts.len(), 3);

        // A later failure in turn a joins its alert without firing.
        let mut more = events.clone();
        more.push(failed(7, Some("a"), "cargo test"));
        assert!(evaluate(&mut g, &more, false).is_empty());
        assert_eq!(g.alerts[0].evidence_seqs, vec![3, 4, 7]);
    }

    #[test]
    fn a_goal_ignores_what_happened_before_it_started() {
        let events = vec![
            failed(3, Some("old"), "npm test"),
            failed(9, Some("new"), "npm test"),
        ];
        let mut g = goal(Some(5));
        let fired = evaluate(&mut g, &events, true);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].turn_id.as_deref(), Some("new"));
        assert!(fired[0].recovered, "found on the catch-up pass");
    }

    #[test]
    fn passed_unknown_and_empty_checks_never_alert() {
        let events: Vec<ProofEvent> = ["passed", "unknown", "empty", "interrupted"]
            .iter()
            .enumerate()
            .map(|(i, s)| {
                ev(
                    i as u64,
                    "check_run",
                    Some("a"),
                    json!({ "command": "npm test", "status": s }),
                )
            })
            .collect();
        let mut g = goal(None);
        assert!(evaluate(&mut g, &events, false).is_empty());
        assert_eq!(g.cursor_seq, Some(3));
    }

    #[test]
    fn demo_timeline_fails_once_and_survives_a_restart() {
        let origin = 1_000_000;
        assert!(demo_events(origin, origin).is_empty());
        let mut g = Goal {
            source: SOURCE_DEMO.into(),
            created_ms: origin,
            ..goal(None)
        };
        // Before the failure: a passing turn, no alert.
        let early = demo_events(origin, origin + DEMO_FAILURE_AT_MS - 1);
        assert!(evaluate(&mut g, &early, false).is_empty());
        // The failure lands: one alert.
        let at = demo_events(origin, origin + DEMO_FAILURE_AT_MS);
        let fired = evaluate(&mut g, &at, false);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].command, "npm test -- totals");
        assert!(fired[0].revision.is_some());
        // "Restart": the goal is reloaded from its serialized form, hours
        // later. The second failed check joins; nothing fires again.
        let mut reloaded: Goal = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        let later = demo_events(origin, origin + 3_600_000);
        assert!(evaluate(&mut reloaded, &later, true).is_empty());
        assert_eq!(reloaded.alerts.len(), 1);
        assert_eq!(reloaded.alerts[0].evidence_seqs.len(), 2);
    }

    #[test]
    fn answers_what_happened_from_the_ledger_and_names_what_is_missing() {
        let origin = 0;
        let events = demo_events(origin, DEMO_FAILURE_AT_MS + 1);
        let mut g = Goal {
            source: SOURCE_DEMO.into(),
            ..goal(None)
        };
        evaluate(&mut g, &events, false);
        let focus = g.alerts[0].id.clone();

        // The turn is still open: the answer says so instead of inventing a diff.
        let ex = answer(&g, &events, "¿Qué pasó en esa ejecución?", Some(&focus));
        assert_eq!(ex.intent, "run_summary");
        assert_eq!(ex.alert_id.as_deref(), Some(focus.as_str()));
        assert!(ex.answer.contains("npm test -- totals"), "{}", ex.answer);
        assert!(ex.answer.contains("código de salida 1"), "{}", ex.answer);
        assert!(ex.answer.contains("Me falta"), "{}", ex.answer);
        assert_eq!(ex.missing.len(), 1);
        assert!(!ex.evidence_seqs.is_empty());

        // Once it closed, the files are there and nothing is missing.
        let all = demo_events(origin, 3_600_000);
        evaluate(&mut g, &all, false);
        let ex = answer(&g, &all, "what happened in that run?", Some(&focus));
        assert!(ex.answer.contains("src/totals.test.ts"), "{}", ex.answer);
        assert!(ex.answer.contains("exit code 1"), "{}", ex.answer);
        assert!(ex.missing.is_empty(), "{:?}", ex.missing);
    }

    #[test]
    fn that_run_is_ambiguous_without_a_focus_and_shows_candidates() {
        let events = vec![
            failed(1, Some("a"), "npm test"),
            failed(2, Some("b"), "cargo test"),
        ];
        let mut g = goal(None);
        evaluate(&mut g, &events, false);
        let ex = answer(&g, &events, "que paso en esa ejecucion", None);
        assert_eq!(ex.intent, "ambiguous");
        assert_eq!(ex.candidates.len(), 2);
        // "the last one" resolves without a focus.
        let ex = answer(&g, &events, "qué pasó en la última ejecución", None);
        assert_eq!(ex.alert_id.as_deref(), Some(g.alerts[1].id.as_str()));
        // A dismissed alert stops being a candidate.
        g.alerts[0].state = "dismissed".into();
        let ex = answer(&g, &events, "what happened in that run", None);
        assert_eq!(ex.alert_id.as_deref(), Some(g.alerts[1].id.as_str()));
    }

    #[test]
    fn no_alert_is_not_reported_as_all_clear_and_unknown_questions_say_so() {
        let g = goal(Some(41));
        let ex = answer(&g, &[], "¿qué pasó en esa ejecución?", None);
        assert_eq!(ex.intent, "no_alerts");
        assert!(ex.answer.contains("seq 41"));
        assert!(ex.answer.contains("no presenció"));
        let ex = answer(&g, &[], "¿el fallo empezó con ese cambio?", None);
        assert_eq!(ex.intent, "unknown");
        let ex = answer(&g, &[], "status", None);
        assert_eq!(ex.intent, "status");
        assert!(ex.answer.contains("Close the PR today"));
    }

    #[test]
    fn notification_names_one_failure_and_counts_several() {
        let f = |cmd: &str| Fired {
            project_root: "/p".into(),
            goal_id: "g".into(),
            goal_title: "Close the PR today".into(),
            alert: Alert {
                command: cmd.into(),
                ..Alert::default()
            },
        };
        let (title, body) = notification_text(&[f("npm test")]);
        assert_eq!(title, "A check failed");
        assert!(body.contains("npm test") && body.contains("Close the PR today"));
        let (title, _) = notification_text(&[f("a"), f("b")]);
        assert_eq!(title, "2 runs failed a check");
    }

    /// The whole loop against a real ledger in an isolated data dir: start,
    /// witness a failing check, tick, "restart", tick again.
    #[test]
    fn follows_a_real_ledger_and_never_repeats_after_restart() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-watch-test-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let root = "/proj/watch";
        let testigo = TestigoService::new();
        let run_check = |ts: i64, cmd: &str, failed: bool| {
            testigo
                .on_tool_result(
                    root,
                    ts,
                    Some("t1"),
                    None,
                    Some("Bash"),
                    Some("output"),
                    false,
                    crate::services::testigo_service::ToolResultIds {
                        command: Some(cmd),
                        failed,
                        exit_code: Some(if failed { 1 } else { 0 }),
                        exit_code_via: Some("payload"),
                        ..Default::default()
                    },
                )
                .unwrap();
        };

        // History that predates the goal — including a failure.
        testigo
            .on_prompt(root, 1, Some("t1"), None, Some("old work"), None, None)
            .unwrap();
        run_check(2, "npm test", true);

        let svc = WatchService::new();
        assert!(svc.list(root).unwrap().is_empty());
        assert!(svc.start(&testigo, root, "   ", false, 10).is_err());
        let g = svc
            .start(&testigo, root, "Close the PR today", false, 10)
            .unwrap();
        assert!(g.cursor_seq.is_some(), "starts at the ledger head");
        assert!(svc.tick_core(&testigo, 11, true).fired.is_empty());

        // A new run fails its tests twice.
        testigo
            .on_prompt(root, 20, Some("t1"), None, Some("fix totals"), None, None)
            .unwrap();
        run_check(21, "cargo test", true);
        run_check(22, "cargo test -p totals", true);
        let out = svc.tick_core(&testigo, 23, false);
        assert_eq!(out.fired.len(), 1, "one alert for the run");
        assert_eq!(out.changed, vec![root.to_string()]);
        assert_eq!(out.fired[0].alert.command, "cargo test");
        assert_eq!(out.fired[0].alert.evidence_seqs.len(), 2);

        // Evidence is the run's own lines; the question is answered from them.
        let alert_id = out.fired[0].alert.id.clone();
        let lines = svc.evidence(&testigo, root, &g.id, &alert_id, 24).unwrap();
        assert!(lines.iter().any(|e| e.kind == "prompt"));
        assert_eq!(lines.iter().filter(|e| e.kind == "check_run").count(), 2);
        let ex = svc
            .ask(
                &testigo,
                root,
                &g.id,
                "¿qué pasó en esa ejecución?",
                Some(&alert_id),
                25,
            )
            .unwrap();
        assert!(ex.answer.contains("fix totals"), "{}", ex.answer);

        // "Restart": fresh services over the same data dir. Nothing repeats,
        // and the goal, its alert and the thread are all still there.
        let testigo2 = TestigoService::new();
        let svc2 = WatchService::new();
        assert!(svc2.tick_core(&testigo2, 30, true).fired.is_empty());
        let goals = svc2.list(root).unwrap();
        assert_eq!(goals[0].title, "Close the PR today");
        assert_eq!(goals[0].alerts.len(), 1);
        assert!(goals[0].alerts[0].notified);
        assert_eq!(goals[0].thread.len(), 1);

        // A run that fails while nothing is ticking ("app closed") is
        // recovered by the next catch-up pass, once.
        testigo2
            .on_prompt(root, 40, Some("t1"), None, Some("try again"), None, None)
            .unwrap();
        testigo2
            .on_tool_result(
                root,
                41,
                Some("t1"),
                None,
                Some("Bash"),
                Some("output"),
                false,
                crate::services::testigo_service::ToolResultIds {
                    command: Some("cargo test"),
                    failed: true,
                    exit_code: Some(101),
                    exit_code_via: Some("payload"),
                    ..Default::default()
                },
            )
            .unwrap();
        let svc3 = WatchService::new();
        let out = svc3.tick_core(&testigo2, 50, true);
        assert_eq!(out.fired.len(), 1);
        assert!(out.fired[0].alert.recovered);
        assert!(svc3.tick_core(&testigo2, 51, false).fired.is_empty());

        // Paused goals are not evaluated; resuming catches up. Stopped is final.
        svc3.set_status(root, &g.id, "paused").unwrap();
        testigo2
            .on_prompt(root, 60, Some("t1"), None, Some("third"), None, None)
            .unwrap();
        run_check_on(&testigo2, root, 61);
        assert!(svc3.tick_core(&testigo2, 62, false).fired.is_empty());
        svc3.set_status(root, &g.id, "active").unwrap();
        assert_eq!(svc3.tick_core(&testigo2, 63, false).fired.len(), 1);
        svc3.set_status(root, &g.id, "stopped").unwrap();
        assert!(svc3.set_status(root, &g.id, "active").is_err());

        // Alert state persists; a corrupt file falls back to the backup.
        let goal = svc3
            .set_alert_state(root, &g.id, &alert_id, "dismissed")
            .unwrap();
        assert_eq!(goal.alerts[0].state, "dismissed");
        let path = WatchService::path("json").unwrap();
        assert!(!WatchService::path("json.tmp").unwrap().exists());
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(svc3.list(root).unwrap().len(), 1, "read from .bak");

        svc3.delete(root, &g.id).unwrap();
        assert!(svc3.list(root).unwrap().is_empty());

        std::env::remove_var("XDG_DATA_HOME");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn run_check_on(testigo: &TestigoService, root: &str, ts: i64) {
        testigo
            .on_tool_result(
                root,
                ts,
                Some("t1"),
                None,
                Some("Bash"),
                Some("output"),
                false,
                crate::services::testigo_service::ToolResultIds {
                    command: Some("npm test"),
                    failed: true,
                    exit_code: Some(1),
                    exit_code_via: Some("payload"),
                    ..Default::default()
                },
            )
            .unwrap();
    }
}
