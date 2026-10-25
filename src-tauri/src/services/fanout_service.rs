//! F9 (COMPETENCIA-2026-10): fan-out — N agents, the same prompt, twin
//! worktrees, one comparison.
//!
//! Each participant works alone on its own `fanout/<id>/<pid>` branch in its
//! own worktree, with `AcceptEdits` (never the user's checkout), one turn.
//! When all are done the console compares what each one actually did: the
//! diff against the base, the checks it ran and their exit codes (from the
//! runner's activity, the same signals F1b folds), and the F1c verdict on
//! its closing words. The human applies one candidate (a `--no-ff` merge
//! onto the base) or discards them all; both go to the ledger.
//!
//! Nothing here judges which diff is better — the console shows the
//! evidence, the human decides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::agent_run::{self, RunSpec};
use crate::services::engine_runner::{Engine, EngineRunner, ToolPolicy};
use crate::services::roundtable_service::Participant;
use crate::services::testigo_service::check_kind;
use crate::services::verification_rules::{self, TurnFacts};
use crate::services::{proc, worktree_service};

/// Candidates kept per project in the store.
const KEEP_PER_PROJECT: usize = 20;
/// Longest closing text carried per candidate.
const SUMMARY_CAP: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanOutConfig {
    pub problem: String,
    /// Two or more; each gets its own worktree and one turn.
    pub participants: Vec<Participant>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateChecks {
    pub passed: u32,
    pub failed: u32,
    /// Ran, exit code not reported by the engine (Claude's stream).
    pub unknown: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub participant: Participant,
    pub branch: String,
    pub worktree: String,
    /// running · done · error
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub summary: String,
    pub files: Vec<String>,
    pub insertions: u32,
    pub deletions: u32,
    pub commands: u32,
    pub checks: CandidateChecks,
    /// F1c status computed from the candidate's own activity and words.
    pub verification: String,
    pub verification_reason: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanOut {
    pub id: String,
    pub project: String,
    pub problem: String,
    pub base_branch: String,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
    pub candidates: Vec<Candidate>,
    /// The participant id the human applied, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<String>,
    /// True once worktrees are gone (applied or discarded).
    #[serde(default)]
    pub closed: bool,
}

impl FanOut {
    pub fn all_done(&self) -> bool {
        self.candidates.iter().all(|c| c.status != "running")
    }
}

/// One line of a runner's activity, as the sink receives it.
#[derive(Debug, Clone)]
pub struct Activity {
    pub kind: String,
    pub label: String,
    pub text: String,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn is_shell(label: &str) -> bool {
    matches!(label, "Bash" | "shell" | "bash")
}

fn is_edit(label: &str) -> bool {
    matches!(
        label,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "edit" | "write" | "patch"
    )
}

/// `"<command> · exit N"` as the runners phrase a finished command.
fn split_exit(text: &str) -> (&str, Option<i64>) {
    match text.rsplit_once(" · exit ") {
        Some((cmd, code)) => (cmd, code.trim().parse().ok()),
        None => (text, None),
    }
}

/// Fold a candidate's activity into F1c facts: commands and edits in order,
/// checks with their exit codes when the engine reported them (Codex,
/// OpenCode) and `unknown` when it did not (Claude's stream shows the call,
/// not the result). The verdict then reads exactly as for a session turn.
pub fn facts_from_activity(activity: &[Activity], summary: &str) -> TurnFacts {
    let mut f = TurnFacts {
        summary: summary.to_string(),
        ..Default::default()
    };
    // A runner that reports results emits `tool` then `tool_result` for the
    // same command: count the result when there is one, the call otherwise.
    let results_seen = activity
        .iter()
        .any(|a| a.kind == "tool_result" && is_shell(&a.label));
    for (i, a) in activity.iter().enumerate() {
        let seq = i as u64 + 1;
        let shell = is_shell(&a.label);
        match a.kind.as_str() {
            "tool" if is_edit(&a.label) => f.last_edit_seq = Some(seq),
            "tool_result" if is_edit(&a.label) => f.last_edit_seq = Some(seq),
            "tool" if shell && !results_seen => {
                f.tool_results += 1;
                f.commands += 1;
                let kind = check_kind(&a.text);
                if !kind.is_empty() {
                    f.checks.unknown += 1;
                    f.last_check_seq = Some(seq);
                    f.gates_ran.insert(kind.to_string(), "unknown".into());
                }
            }
            "tool_result" if shell => {
                f.tool_results += 1;
                f.commands += 1;
                let (cmd, exit) = split_exit(&a.text);
                if exit.is_some_and(|c| c != 0) {
                    f.failed_commands += 1;
                }
                let kind = check_kind(cmd);
                if !kind.is_empty() {
                    f.last_check_seq = Some(seq);
                    let status = match exit {
                        Some(0) => {
                            f.checks.passed += 1;
                            f.last_good_check_seq = Some(seq);
                            "passed"
                        }
                        Some(_) => {
                            f.checks.failed += 1;
                            "failed"
                        }
                        None => {
                            f.checks.unknown += 1;
                            "unknown"
                        }
                    };
                    let worse = |a: &str, b: &str| {
                        let rank = |s: &str| match s {
                            "failed" => 2,
                            "unknown" => 1,
                            _ => 0,
                        };
                        rank(a) > rank(b)
                    };
                    let e = f
                        .gates_ran
                        .entry(kind.to_string())
                        .or_insert_with(|| status.to_string());
                    if worse(status, e) {
                        *e = status.to_string();
                    }
                }
            }
            _ => {}
        }
    }
    f
}

fn git(repo: &Path, args: &[&str]) -> AppResult<std::process::Output> {
    Ok(proc::command("git").args(args).current_dir(repo).output()?)
}

/// Files changed and the +/- counts of `base..branch`.
pub fn diff_summary(repo: &Path, base: &str, branch: &str) -> (Vec<String>, u32, u32) {
    let range = format!("{base}..{branch}");
    let files = git(repo, &["diff", "--name-only", &range])
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let (mut ins, mut del) = (0u32, 0u32);
    if let Ok(o) = git(repo, &["diff", "--numstat", &range]) {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let mut it = line.split('\t');
            let a = it.next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
            let b = it.next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
            ins += a;
            del += b;
        }
    }
    (files, ins, del)
}

fn worktree_root(id: &str) -> PathBuf {
    std::env::temp_dir().join("agent-console-fanouts").join(id)
}

fn cap(s: &str, n: usize) -> String {
    crate::services::text::cap_chars(s, n)
}

/// Where a project's fan-outs are kept (`fanouts.json` under the data dir).
fn store_path() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("no data_local dir".into()))?
        .join("agent-console");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("fanouts.json"))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    fanouts: Vec<FanOut>,
}

fn load_store() -> StoreFile {
    let Ok(p) = store_path() else {
        return StoreFile::default();
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_store(file: &StoreFile) -> AppResult<()> {
    let p = store_path()?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(file).map_err(|e| AppError::Other(e.to_string()))?,
    )?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// Live fan-outs plus the on-disk history.
#[derive(Default)]
pub struct FanOutService {
    runs: Mutex<HashMap<String, Arc<Mutex<FanOut>>>>,
    /// Serializes every write of `fanouts.json` — the service's and the
    /// workers' (they get a clone).
    disk: Arc<Mutex<()>>,
}

/// Most activity lines kept per candidate (tool calls and results only).
const ACTIVITY_CAP: usize = 5_000;
/// Participant ids become branch and path components: a short safe charset.
fn safe_participant_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A record read from disk for a fan-out that is not live: a candidate
/// still `running` was interrupted (the console exited mid-run) — say so,
/// so the comparison closes instead of spinning forever.
fn settle_stale(mut fo: FanOut) -> FanOut {
    let mut touched = false;
    for c in &mut fo.candidates {
        if c.status == "running" {
            c.status = "error".into();
            c.error = Some("interrupted: the console exited before this candidate finished".into());
            touched = true;
        }
    }
    if touched && fo.finished_at_ms.is_none() {
        fo.finished_at_ms = Some(now_ms());
    }
    fo
}

/// Called whenever a candidate finishes: `(id, finished)` where `finished`
/// is true exactly once per fan-out — on the call that completed it. The
/// Tauri layer emits `fanout://changed` on every call and witnesses
/// `fanout_result` on the finishing one.
pub type Notify = Arc<dyn Fn(&str, bool) + Send + Sync>;

impl FanOutService {
    pub fn new() -> Self {
        Self::default()
    }

    fn persist(&self, fo: &FanOut) {
        let _g = self.disk.lock().unwrap();
        persist_locked(fo);
    }
}

/// Write one fan-out into the store. The caller holds the disk lock.
fn persist_locked(fo: &FanOut) {
    {
        let mut file = load_store();
        file.fanouts.retain(|f| f.id != fo.id);
        file.fanouts.push(fo.clone());
        // Newest first per project, bounded.
        file.fanouts
            .sort_by_key(|f| std::cmp::Reverse(f.started_at_ms));
        let mut seen: HashMap<String, usize> = HashMap::new();
        file.fanouts.retain(|f| {
            let n = seen.entry(f.project.clone()).or_insert(0);
            *n += 1;
            *n <= KEEP_PER_PROJECT
        });
        if let Err(e) = save_store(&file) {
            tracing::warn!("fanout: failed to persist {}: {e}", fo.id);
        }
    }
}

impl FanOutService {
    /// Every fan-out of a project, newest first (live ones as they stand now).
    pub fn list(&self, project: &str) -> Vec<FanOut> {
        let live: HashMap<String, FanOut> = self
            .runs
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.lock().unwrap().clone()))
            .collect();
        let mut out: Vec<FanOut> = load_store()
            .fanouts
            .into_iter()
            .filter(|f| f.project == project)
            .map(|f| live.get(&f.id).cloned().unwrap_or_else(|| settle_stale(f)))
            .collect();
        for (id, f) in live {
            if f.project == project && !out.iter().any(|o| o.id == id) {
                out.push(f);
            }
        }
        out.sort_by_key(|f| std::cmp::Reverse(f.started_at_ms));
        out
    }

    pub fn get(&self, id: &str) -> Option<FanOut> {
        if let Some(l) = self.runs.lock().unwrap().get(id) {
            return Some(l.lock().unwrap().clone());
        }
        load_store()
            .fanouts
            .into_iter()
            .find(|f| f.id == id)
            .map(settle_stale)
    }

    /// Stand up one worktree per participant and run them all at once, each
    /// on its own thread. Returns when the worktrees exist; the candidates
    /// fill in as they finish. `runner_for` is injected (tests fake it).
    pub fn start(
        &self,
        repo: &Path,
        config: FanOutConfig,
        runner_for: fn(Engine) -> &'static dyn EngineRunner,
        notify: Notify,
    ) -> AppResult<String> {
        if config.problem.trim().is_empty() {
            return Err(AppError::InvalidArgument("problem is empty".into()));
        }
        if config.participants.len() < 2 {
            return Err(AppError::InvalidArgument(
                "a fan-out needs at least two participants".into(),
            ));
        }
        for p in &config.participants {
            if !safe_participant_id(&p.id) {
                return Err(AppError::InvalidArgument(format!(
                    "participant id `{}` is not safe for a branch name (letters, digits, - and _)",
                    p.id
                )));
            }
            if !crate::services::roundtable_service::is_safe_model(&p.model) {
                return Err(AppError::InvalidArgument(format!(
                    "invalid model value for {}",
                    p.name
                )));
            }
        }
        {
            let mut ids: Vec<&str> = config.participants.iter().map(|p| p.id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            if ids.len() != config.participants.len() {
                return Err(AppError::InvalidArgument(
                    "participant ids must be unique".into(),
                ));
            }
        }
        let base = worktree_service::current_branch(repo)?;
        let id = format!(
            "fo-{}-{}",
            now_ms(),
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        );
        let mut candidates: Vec<Candidate> = Vec::with_capacity(config.participants.len());
        for p in &config.participants {
            let branch = format!("fanout/{id}/{}", p.id);
            let dest = worktree_root(&id).join(&p.id);
            // One failed worktree tears the whole fan-out down: a comparison
            // with a missing candidate is not the comparison asked for.
            if let Err(e) = worktree_service::create(repo, &dest, &branch, Some(&base)) {
                for c in &candidates {
                    let _ =
                        worktree_service::discard(repo, Path::new(&c.worktree), &c.branch, true);
                }
                return Err(e);
            }
            candidates.push(Candidate {
                participant: p.clone(),
                branch,
                worktree: dest.to_string_lossy().to_string(),
                status: "running".into(),
                error: None,
                summary: String::new(),
                files: Vec::new(),
                insertions: 0,
                deletions: 0,
                commands: 0,
                checks: CandidateChecks::default(),
                verification: "unwitnessed".into(),
                verification_reason: String::new(),
                tokens: 0,
                cost_usd: 0.0,
                duration_ms: 0,
            });
        }
        let fo = FanOut {
            id: id.clone(),
            project: repo.to_string_lossy().to_string(),
            problem: config.problem.trim().to_string(),
            base_branch: base.clone(),
            started_at_ms: now_ms(),
            finished_at_ms: None,
            candidates,
            applied: None,
            closed: false,
        };
        let shared = Arc::new(Mutex::new(fo.clone()));
        self.runs.lock().unwrap().insert(id.clone(), shared.clone());
        self.persist(&fo);
        for (idx, p) in config.participants.iter().enumerate() {
            let shared = shared.clone();
            let notify = notify.clone();
            let p = p.clone();
            let prompt = fo.problem.clone();
            let repo = repo.to_path_buf();
            let base = base.clone();
            let (wt, branch) = {
                let c = &fo.candidates[idx];
                (PathBuf::from(&c.worktree), c.branch.clone())
            };
            let id = id.clone();
            let disk = self.disk.clone();
            std::thread::spawn(move || {
                // Resolved on the thread: the runners are `'static` but not
                // `Sync`, so the function pointer travels, not the reference.
                let runner = runner_for(p.engine);
                let started = now_ms();
                let activity: Mutex<Vec<Activity>> = Mutex::new(Vec::new());
                // Only what the facts read: tool calls and results. Streamed
                // text deltas would be thousands of strings kept for nothing.
                let sink = |kind: &str, label: &str, text: &str| {
                    if kind != "tool" && kind != "tool_result" {
                        return;
                    }
                    let mut a = activity.lock().unwrap();
                    if a.len() < ACTIVITY_CAP {
                        a.push(Activity {
                            kind: kind.into(),
                            label: label.into(),
                            text: text.into(),
                        });
                    }
                };
                let outcome = agent_run::run_with_runner(
                    &RunSpec {
                        engine: p.engine,
                        cwd: &wt,
                        prompt: &prompt,
                        policy: ToolPolicy::AcceptEdits,
                        model: Some(&p.model),
                        resume: None,
                        idle_timeout: agent_run::default_idle_timeout(),
                        on_activity: Some(&sink),
                    },
                    runner,
                );
                let activity = activity.into_inner().unwrap_or_default();
                // Whatever the agent left uncommitted is the candidate.
                let _ = worktree_service::commit_all(&wt, &format!("fan-out {id}: {}", p.name));
                let (files, ins, del) = diff_summary(&repo, &base, &branch);
                let mut fo = shared.lock().unwrap();
                // Discarded while this ran: the worktree is gone and the
                // record is closed — nothing to persist, nobody to tell.
                if fo.closed {
                    return;
                }
                let c = &mut fo.candidates[idx];
                c.files = files;
                c.insertions = ins;
                c.deletions = del;
                c.duration_ms = now_ms().saturating_sub(started);
                match outcome {
                    Ok(o) => {
                        c.summary = cap(o.text.trim(), SUMMARY_CAP);
                        c.tokens = o.tokens;
                        c.cost_usd = o.cost_usd;
                        c.status = "done".into();
                    }
                    Err(e) => {
                        c.error = Some(e.to_string());
                        c.status = "error".into();
                    }
                }
                let facts = facts_from_activity(&activity, &c.summary);
                let claims = verification_rules::claims(&facts.summary);
                let (status, reason) = verification_rules::verdict(&facts, &claims);
                c.commands = facts.commands;
                c.checks = CandidateChecks {
                    passed: facts.checks.passed,
                    failed: facts.checks.failed,
                    unknown: facts.checks.unknown,
                };
                c.verification = status.as_str().into();
                c.verification_reason = reason;
                // Exactly one worker completes the fan-out: the one that sets
                // finished_at_ms. That flag is what the ledger line hangs on.
                let finished = fo.all_done() && fo.finished_at_ms.is_none();
                if finished {
                    fo.finished_at_ms = Some(now_ms());
                }
                drop(fo);
                // Snapshot under the disk lock so two workers can never
                // write an older state after a newer one.
                {
                    let _g = disk.lock().unwrap();
                    let snapshot = shared.lock().unwrap().clone();
                    persist_locked(&snapshot);
                }
                notify(&id, finished);
            });
        }
        Ok(id)
    }

    /// The payload `fanout_result` carries — the comparison in one line.
    pub fn result_payload(fo: &FanOut) -> Value {
        json!({
            "id": fo.id,
            "problem": cap(&fo.problem, 300),
            "baseBranch": fo.base_branch,
            "candidates": fo.candidates.iter().map(|c| json!({
                "participant": c.participant.id,
                "name": c.participant.name,
                "engine": c.participant.engine,
                "model": c.participant.model,
                "branch": c.branch,
                "status": c.status,
                "files": c.files.len(),
                "insertions": c.insertions,
                "deletions": c.deletions,
                "checks": c.checks,
                "verification": c.verification,
                "tokens": c.tokens,
                "costUsd": c.cost_usd,
            })).collect::<Vec<_>>(),
        })
    }

    /// Apply one candidate: merge its branch onto the base (`--no-ff`), then
    /// tear every worktree down (branches stay, like a room's). Refuses
    /// while any candidate still runs — the comparison is not complete.
    pub fn apply(
        &self,
        repo: &Path,
        id: &str,
        participant_id: &str,
    ) -> AppResult<worktree_service::MergeOutcome> {
        // The live handle first; a closed fan-out comes from disk. Two
        // statements on purpose: `get` locks `runs` too, and a guard kept
        // alive across the fallback would deadlock on itself.
        let live = self.runs.lock().unwrap().get(id).cloned();
        let shared = match live {
            Some(l) => l,
            None => Arc::new(Mutex::new(
                self.get(id)
                    .ok_or_else(|| AppError::NotFound(format!("fan-out {id}")))?,
            )),
        };
        let (candidate, base, done, closed) = {
            let fo = shared.lock().unwrap();
            (
                fo.candidates
                    .iter()
                    .find(|c| c.participant.id == participant_id)
                    .cloned(),
                fo.base_branch.clone(),
                fo.all_done(),
                fo.closed,
            )
        };
        let candidate = candidate.ok_or_else(|| {
            AppError::InvalidArgument(format!("no candidate {participant_id} in {id}"))
        })?;
        if closed {
            return Err(AppError::InvalidArgument(
                "this fan-out is closed (applied or discarded)".into(),
            ));
        }
        if !done {
            return Err(AppError::InvalidArgument(
                "a candidate is still running — wait for the comparison".into(),
            ));
        }
        let outcome = worktree_service::merge_back(
            repo,
            Path::new(&candidate.worktree),
            &candidate.branch,
            &base,
            &format!(
                "Apply fan-out candidate {} ({} {})",
                candidate.participant.name,
                candidate.participant.engine.bin_name(),
                candidate.participant.model
            ),
        )?;
        if outcome.merged {
            let snapshot = {
                let mut fo = shared.lock().unwrap();
                for c in &fo.candidates {
                    let _ =
                        worktree_service::discard(repo, Path::new(&c.worktree), &c.branch, false);
                }
                fo.applied = Some(participant_id.to_string());
                fo.closed = true;
                fo.clone()
            };
            self.persist(&snapshot);
            self.runs.lock().unwrap().remove(id);
        }
        Ok(outcome)
    }

    /// Discard every candidate: worktrees and branches gone. A candidate
    /// still running loses its checkout; its worker sees `closed` and
    /// persists nothing — the record stays discarded.
    pub fn discard(&self, repo: &Path, id: &str) -> AppResult<()> {
        let live = self.runs.lock().unwrap().get(id).cloned();
        let fo = match &live {
            Some(l) => {
                let mut fo = l.lock().unwrap();
                fo.closed = true;
                fo.clone()
            }
            None => {
                let mut fo = self
                    .get(id)
                    .ok_or_else(|| AppError::NotFound(format!("fan-out {id}")))?;
                fo.closed = true;
                fo
            }
        };
        for c in &fo.candidates {
            let _ = worktree_service::discard(repo, Path::new(&c.worktree), &c.branch, true);
        }
        self.persist(&fo);
        self.runs.lock().unwrap().remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::engine_runner::{RunCtx, TurnOutput};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn act(kind: &str, label: &str, text: &str) -> Activity {
        Activity {
            kind: kind.into(),
            label: label.into(),
            text: text.into(),
        }
    }

    /// The same words, three engines' worth of evidence: a reported exit 0
    /// after the edit verifies, a reported exit 1 contradicts, a call with
    /// no result (Claude's stream) leaves the claim unverified.
    #[test]
    fn facts_from_activity_read_exit_codes_and_edit_order() {
        let a = [
            act("tool", "Edit", "src/x.ts"),
            act("tool", "shell", "npm test"),
            act("tool_result", "shell", "npm test · exit 0"),
        ];
        let f = facts_from_activity(&a, "Done, all tests pass.");
        assert_eq!(f.commands, 1, "call + result count once");
        assert_eq!(f.checks.passed, 1);
        assert!(f.last_good_check_seq > f.last_edit_seq);
        let (st, _) = verification_rules::verdict(&f, &verification_rules::claims(&f.summary));
        assert_eq!(st.as_str(), "verified");

        let b = [
            act("tool", "Edit", "src/x.ts"),
            act("tool_result", "shell", "npm test · exit 1"),
        ];
        let f = facts_from_activity(&b, "All tests pass.");
        assert_eq!(f.checks.failed, 1);
        assert_eq!(f.failed_commands, 1);
        let (st, _) = verification_rules::verdict(&f, &verification_rules::claims(&f.summary));
        assert_eq!(st.as_str(), "claim_contradicted");

        let c = [
            act("tool", "Bash", "cargo test"),
            act("tool", "Edit", "a.rs"),
        ];
        let f = facts_from_activity(&c, "tests pass");
        assert_eq!(f.checks.unknown, 1);
        assert_eq!(f.gates_ran.get("test").map(String::as_str), Some("unknown"));
        let (st, _) = verification_rules::verdict(&f, &verification_rules::claims(&f.summary));
        assert_ne!(st.as_str(), "verified");

        let f = facts_from_activity(&[], "");
        let (st, _) = verification_rules::verdict(&f, &[]);
        assert_eq!(st.as_str(), "no_claim");
    }

    fn init_repo(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo =
            std::env::temp_dir().join(format!("ac-fanout-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&repo).unwrap();
        let g = |args: &[&str]| {
            let o = proc::command("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
        };
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "T"]);
        g(&["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README.md"), "base\n").unwrap();
        g(&["add", "README.md"]);
        g(&["commit", "-qm", "base"]);
        repo
    }

    /// A fake engine: writes one file named after the model into its cwd,
    /// reports a check whose exit code the model name dictates, and claims
    /// green. So "opus" verifies and "sonnet" contradicts — the comparison
    /// has something to compare.
    struct Scripted;
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    impl EngineRunner for Scripted {
        fn run(
            &self,
            ctx: &RunCtx,
            on_activity: &crate::services::engine_runner::ActivitySink,
        ) -> AppResult<TurnOutput> {
            RUNS.fetch_add(1, Ordering::SeqCst);
            assert_eq!(ctx.tools, ToolPolicy::AcceptEdits);
            let name = format!("by-{}.txt", ctx.model);
            std::fs::write(ctx.cwd.join(&name), format!("{}\n", ctx.prompt)).unwrap();
            on_activity("tool", "Edit", &name);
            let exit = if ctx.model == "opus" { 0 } else { 1 };
            on_activity("tool_result", "shell", &format!("npm test · exit {exit}"));
            Ok(TurnOutput {
                text: format!("Wrote {name}. All tests pass."),
                session_id: None,
                tokens: 10,
                cost_usd: 0.01,
            })
        }
    }
    fn scripted(_: Engine) -> &'static dyn EngineRunner {
        &Scripted
    }

    fn participant(id: &str, model: &str) -> Participant {
        Participant {
            id: id.into(),
            name: id.to_uppercase(),
            engine: Engine::Claude,
            model: model.into(),
            role: String::new(),
            roles: Vec::new(),
        }
    }

    /// Two agents, one prompt, twin worktrees: each lands on its own branch
    /// with its own diff and verdict; applying one merges it onto the base
    /// and tears the worktrees down; the other's branch survives.
    #[test]
    fn fan_out_runs_twins_compares_and_applies_the_pick() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("ac-fanout-data-{nanos}"));
        std::fs::create_dir_all(&data).unwrap();
        std::env::set_var("XDG_DATA_HOME", &data);
        let repo = init_repo("twins");
        let svc = FanOutService::new();
        let (tx, rx) = std::sync::mpsc::channel::<(String, bool)>();
        let notify: Notify = Arc::new(move |id: &str, finished: bool| {
            let _ = tx.send((id.to_string(), finished));
        });
        let id = svc
            .start(
                &repo,
                FanOutConfig {
                    problem: "add a greeting file".into(),
                    participants: vec![participant("p1", "opus"), participant("p2", "sonnet")],
                },
                scripted,
                notify,
            )
            .unwrap();
        // Two notifications: one per finished candidate; exactly one says
        // "this one completed the fan-out".
        let mut finished_flags = Vec::new();
        for _ in 0..2 {
            let (_, f) = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
            finished_flags.push(f);
        }
        assert_eq!(finished_flags.iter().filter(|f| **f).count(), 1);
        let fo = svc.get(&id).unwrap();
        assert!(fo.all_done());
        assert!(fo.finished_at_ms.is_some());
        assert_eq!(fo.base_branch, "main");
        let by = |pid: &str| {
            fo.candidates
                .iter()
                .find(|c| c.participant.id == pid)
                .unwrap()
                .clone()
        };
        let a = by("p1");
        let b = by("p2");
        assert_eq!(a.status, "done");
        assert_eq!(a.files, vec!["by-opus.txt"]);
        assert_eq!((a.insertions, a.deletions), (1, 0));
        assert_eq!(a.checks.passed, 1);
        assert_eq!(a.verification, "verified");
        assert_eq!(b.files, vec!["by-sonnet.txt"]);
        assert_eq!(b.checks.failed, 1);
        assert_eq!(b.verification, "claim_contradicted");
        assert_eq!(a.tokens, 10);
        assert!(svc.list(&repo.to_string_lossy()).iter().any(|f| f.id == id));

        // Apply the verified one.
        let out = svc.apply(&repo, &id, "p1").unwrap();
        assert!(out.merged, "{}", out.detail);
        assert!(repo.join("by-opus.txt").exists());
        assert!(!repo.join("by-sonnet.txt").exists());
        let after = svc.get(&id).unwrap();
        assert_eq!(after.applied.as_deref(), Some("p1"));
        assert!(after.closed);
        assert!(!Path::new(&a.worktree).exists(), "worktrees are gone");
        // The other candidate's branch survives for the record.
        let o = proc::command("git")
            .args(["branch", "--list", &b.branch])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains(&b.branch));
        assert!(svc.apply(&repo, &id, "p2").is_err(), "closed");
    }

    #[test]
    fn a_fan_out_needs_two_participants_and_a_problem() {
        let _env = crate::test_support::lock_env();
        let repo = init_repo("cfg");
        let svc = FanOutService::new();
        let notify: Notify = Arc::new(|_: &str, _: bool| {});
        // Ids become branch and path components; models reach argv.
        let mut bad_id = participant("p1", "opus");
        bad_id.id = "../../x".into();
        assert!(svc
            .start(
                &repo,
                FanOutConfig {
                    problem: "x".into(),
                    participants: vec![bad_id, participant("p2", "sonnet")],
                },
                scripted,
                notify.clone(),
            )
            .is_err());
        assert!(svc
            .start(
                &repo,
                FanOutConfig {
                    problem: "x".into(),
                    participants: vec![
                        participant("p1", "opus; rm -rf"),
                        participant("p2", "sonnet")
                    ],
                },
                scripted,
                notify.clone(),
            )
            .is_err());
        assert!(
            svc.start(
                &repo,
                FanOutConfig {
                    problem: "x".into(),
                    participants: vec![participant("p1", "opus"), participant("p1", "sonnet")],
                },
                scripted,
                notify.clone(),
            )
            .is_err(),
            "duplicate ids"
        );
        assert!(svc
            .start(
                &repo,
                FanOutConfig {
                    problem: " ".into(),
                    participants: vec![participant("p1", "opus"), participant("p2", "sonnet")],
                },
                scripted,
                notify.clone(),
            )
            .is_err());
        assert!(svc
            .start(
                &repo,
                FanOutConfig {
                    problem: "x".into(),
                    participants: vec![participant("p1", "opus")],
                },
                scripted,
                notify,
            )
            .is_err());
    }

    /// A slow fake: lets the test discard mid-run.
    struct Slow;
    static SLOW_DONE: AtomicUsize = AtomicUsize::new(0);
    impl EngineRunner for Slow {
        fn run(
            &self,
            ctx: &RunCtx,
            on_activity: &crate::services::engine_runner::ActivitySink,
        ) -> AppResult<TurnOutput> {
            std::thread::sleep(std::time::Duration::from_millis(600));
            let _ = std::fs::write(ctx.cwd.join("late.txt"), "late\n");
            on_activity("tool", "Edit", "late.txt");
            SLOW_DONE.fetch_add(1, Ordering::SeqCst);
            Ok(TurnOutput {
                text: "done".into(),
                session_id: None,
                tokens: 1,
                cost_usd: 0.0,
            })
        }
    }
    fn slow(_: Engine) -> &'static dyn EngineRunner {
        &Slow
    }

    /// Discarding while candidates run stays discarded: the late workers
    /// see `closed`, persist nothing and notify nobody.
    #[test]
    fn discard_while_running_is_not_undone_by_late_workers() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("ac-fanout-data-d-{nanos}"));
        std::fs::create_dir_all(&data).unwrap();
        std::env::set_var("XDG_DATA_HOME", &data);
        let repo = init_repo("discard");
        let svc = FanOutService::new();
        let notified = Arc::new(AtomicUsize::new(0));
        let n2 = notified.clone();
        let notify: Notify = Arc::new(move |_: &str, _: bool| {
            n2.fetch_add(1, Ordering::SeqCst);
        });
        let before = SLOW_DONE.load(Ordering::SeqCst);
        let id = svc
            .start(
                &repo,
                FanOutConfig {
                    problem: "slow".into(),
                    participants: vec![participant("p1", "opus"), participant("p2", "sonnet")],
                },
                slow,
                notify,
            )
            .unwrap();
        svc.discard(&repo, &id).unwrap();
        assert!(svc.get(&id).unwrap().closed);
        // Let both workers finish their (now homeless) runs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while SLOW_DONE.load(Ordering::SeqCst) < before + 2 && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        let after = svc.get(&id).unwrap();
        assert!(
            after.closed,
            "a late worker must not reopen a discarded fan-out"
        );
        assert_eq!(
            notified.load(Ordering::SeqCst),
            0,
            "nothing to announce once discarded"
        );
        assert!(svc.apply(&repo, &id, "p1").is_err());
    }

    /// A record left `running` by a crash reads back as interrupted, so the
    /// comparison closes instead of spinning forever.
    #[test]
    fn a_fan_out_interrupted_by_a_restart_settles_as_error() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("ac-fanout-data-s-{nanos}"));
        std::fs::create_dir_all(&data).unwrap();
        std::env::set_var("XDG_DATA_HOME", &data);
        let svc = FanOutService::new();
        let mut c = Candidate {
            participant: participant("p1", "opus"),
            branch: "fanout/x/p1".into(),
            worktree: "/tmp/none".into(),
            status: "running".into(),
            error: None,
            summary: String::new(),
            files: Vec::new(),
            insertions: 0,
            deletions: 0,
            commands: 0,
            checks: CandidateChecks::default(),
            verification: "unwitnessed".into(),
            verification_reason: String::new(),
            tokens: 0,
            cost_usd: 0.0,
            duration_ms: 0,
        };
        let done = {
            let mut d = c.clone();
            d.participant = participant("p2", "sonnet");
            d.status = "done".into();
            d
        };
        c.status = "running".into();
        let fo = FanOut {
            id: "fo-stale".into(),
            project: "/p".into(),
            problem: "x".into(),
            base_branch: "main".into(),
            started_at_ms: 1,
            finished_at_ms: None,
            candidates: vec![c, done],
            applied: None,
            closed: false,
        };
        svc.persist(&fo);
        let read = svc.get("fo-stale").unwrap();
        assert_eq!(read.candidates[0].status, "error");
        assert!(read.candidates[0]
            .error
            .as_deref()
            .unwrap()
            .contains("interrupted"));
        assert_eq!(read.candidates[1].status, "done");
        assert!(read.all_done());
        assert!(read.finished_at_ms.is_some());
        assert_eq!(svc.list("/p")[0].candidates[0].status, "error");
    }
}
