//! D12: the harness improves itself — proposes, never merges. "Model frozen,
//! harness evolving", the autoresearch shape with the study's gates:
//!
//! - G1 the metric is declared before the run and computed by this code,
//!   which the agent cannot edit (it lives in the console, not the repo):
//!   pass rate on held-out tasks.
//! - G2 the held-out tasks live outside the repo (the console's data dir),
//!   are never shown to the agent, and rotate: each experiment runs a
//!   window of them, after the agent has declared its change done.
//! - G3 a fixed budget (minutes of silence, tokens); the stop is recorded.
//! - G4 every change is a branch in an isolated worktree with lineage
//!   (`Harness-Experiment` / `Harness-Parent` trailers, `experiment_*`
//!   ledger events); a kept one is pushed and gets a PR link. Nothing here
//!   merges. A change outside the editable allowlist (the evaluator, tests,
//!   source) is rejected after the fact by a diff the agent cannot avoid.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::agent_run::{self, RunSpec};
use crate::services::engine_runner::{Engine, EngineRunner, ToolPolicy};
use crate::services::testigo_service::TestigoService;
use crate::services::{fs_names, proc, worktree_service};

/// What an experiment may touch: the harness, not the code it drives.
pub const EDITABLE: &[&str] = &[
    "CLAUDE.md",
    "AGENTS.md",
    ".claude/skills/",
    ".claude/agents/",
    ".claude/hooks/",
    ".claude/settings.json",
];
/// Held-out tasks run per experiment (a rotating window).
pub const WINDOW: usize = 3;

pub fn is_editable(path: &str, editable: &[String]) -> bool {
    let p = path.replace('\\', "/");
    editable.iter().any(|e| {
        if let Some(dir) = e.strip_suffix('/') {
            p.starts_with(&format!("{dir}/"))
        } else {
            p == *e
        }
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeldOutTask {
    pub id: String,
    /// What the agent is asked to do under the candidate harness.
    pub prompt: String,
    /// Shell command run afterwards in the worktree; exit 0 = pass.
    pub check: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeldOut {
    pub version: u32,
    pub tasks: Vec<HeldOutTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Budget {
    pub idle_minutes: u64,
    pub max_tokens: u64,
}
impl Default for Budget {
    fn default() -> Self {
        Budget {
            idle_minutes: 10,
            max_tokens: 400_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LabConfig {
    /// The one metric, declared here, computed by the console.
    pub metric: String,
    pub editable: Vec<String>,
    pub budget: Budget,
    /// A hint for the agent about where to look (optional, human-written).
    pub focus: String,
}
impl Default for LabConfig {
    fn default() -> Self {
        LabConfig {
            metric: "heldout_pass_rate".into(),
            editable: EDITABLE.iter().map(|s| s.to_string()).collect(),
            budget: Budget::default(),
            focus: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub passed: usize,
    pub total: usize,
    pub tasks: Vec<TaskResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub id: String,
    pub passed: bool,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Experiment {
    pub id: String,
    pub parent_id: Option<String>,
    pub created_ms: i64,
    pub finished_ms: Option<i64>,
    pub metric: String,
    pub hypothesis: String,
    pub branch: String,
    /// planned | running | kept | reverted | rejected | failed
    pub status: String,
    pub stop_reason: Option<String>,
    pub heldout_version: u32,
    pub selected_tasks: Vec<String>,
    pub baseline: Option<Score>,
    pub score: Option<Score>,
    pub files_changed: Vec<String>,
    pub outside_files: Vec<String>,
    pub tokens: u64,
    pub cost_usd: f64,
    pub pr_url: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LabState {
    pub config: LabConfig,
    pub heldout: HeldOut,
    pub experiments: Vec<Experiment>,
    /// Baselines by "<heldout version>:<task ids>" — measured once.
    #[serde(default)]
    pub baselines: HashMap<String, Score>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabSummary {
    pub config: LabConfig,
    pub heldout: HeldOut,
    pub experiments: Vec<Experiment>,
    /// Experiments that never touched anything outside the allowlist — the
    /// study's "nights without an edit to the evaluator".
    pub clean_runs: usize,
    pub kept: usize,
}

fn lab_dir(project_root: &str) -> AppResult<PathBuf> {
    Ok(dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("no data_local dir".into()))?
        .join("agent-console")
        .join("harness-lab")
        .join(fs_names::project_slug(Path::new(project_root))))
}

pub fn load(project_root: &str) -> AppResult<LabState> {
    let path = lab_dir(project_root)?.join("lab.json");
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| AppError::Other(format!("lab.json: {e}"))),
        Err(_) => Ok(LabState::default()),
    }
}

pub fn save(project_root: &str, state: &LabState) -> AppResult<()> {
    let dir = lab_dir(project_root)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("lab.json");
    let tmp = dir.join("lab.json.tmp");
    fs::write(
        &tmp,
        serde_json::to_string_pretty(state).map_err(|e| AppError::Other(e.to_string()))?,
    )?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn summary(project_root: &str) -> AppResult<LabSummary> {
    let st = load(project_root)?;
    let clean_runs = st
        .experiments
        .iter()
        .filter(|e| e.finished_ms.is_some() && e.outside_files.is_empty())
        .count();
    let kept = st.experiments.iter().filter(|e| e.status == "kept").count();
    Ok(LabSummary {
        config: st.config,
        heldout: st.heldout,
        experiments: st.experiments,
        clean_runs,
        kept,
    })
}

pub fn set_config(project_root: &str, config: LabConfig) -> AppResult<LabConfig> {
    let mut st = load(project_root)?;
    let mut config = config;
    config.editable = config
        .editable
        .iter()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    if config.editable.is_empty() {
        return Err(AppError::Other(
            "at least one editable path is required".into(),
        ));
    }
    if config.budget.idle_minutes == 0 || config.budget.max_tokens == 0 {
        return Err(AppError::Other("budget must be positive".into()));
    }
    config.metric = "heldout_pass_rate".into();
    st.config = config.clone();
    save(project_root, &st)?;
    Ok(config)
}

/// Replace the held-out tasks (a new version: baselines are invalidated).
pub fn set_heldout(project_root: &str, tasks: Vec<HeldOutTask>) -> AppResult<HeldOut> {
    let mut st = load(project_root)?;
    let tasks: Vec<HeldOutTask> = tasks
        .into_iter()
        .filter(|t| !t.prompt.trim().is_empty() && !t.check.trim().is_empty())
        .collect();
    st.heldout = HeldOut {
        version: st.heldout.version + 1,
        tasks,
    };
    st.baselines.clear();
    save(project_root, &st)?;
    Ok(st.heldout)
}

/// The rotating window: experiment number `n` runs WINDOW tasks starting at
/// `n % len`, so over a week every task is seen and no single run sees all.
pub fn select_tasks(tasks: &[HeldOutTask], n: usize) -> Vec<HeldOutTask> {
    if tasks.is_empty() {
        return Vec::new();
    }
    let k = WINDOW.min(tasks.len());
    (0..k)
        .map(|i| tasks[(n + i) % tasks.len()].clone())
        .collect()
}

fn git(repo: &Path, args: &[&str]) -> AppResult<std::process::Output> {
    Ok(proc::command("git").args(args).current_dir(repo).output()?)
}

fn changed_files(worktree: &Path) -> AppResult<Vec<String>> {
    let out = git(
        worktree,
        &["status", "--porcelain", "--untracked-files=all"],
    )?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.get(3..).map(|p| p.trim().trim_matches('"').to_string()))
        .filter(|p| !p.is_empty())
        .collect())
}

fn reset_worktree(worktree: &Path, to: &str) {
    let _ = git(worktree, &["reset", "--hard", to]);
    let _ = git(worktree, &["clean", "-fdq"]);
}

fn run_check(worktree: &Path, check: &str) -> (bool, String) {
    #[cfg(windows)]
    let out = proc::command("cmd")
        .args(["/C", check])
        .current_dir(worktree)
        .output();
    #[cfg(not(windows))]
    let out = proc::command("sh")
        .args(["-c", check])
        .current_dir(worktree)
        .output();
    match out {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            (
                o.status.success(),
                text.chars()
                    .rev()
                    .take(400)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect(),
            )
        }
        Err(e) => (false, e.to_string()),
    }
}

/// Run the held-out window under the worktree's harness: one headless turn
/// per task, then its check; the worktree is reset to `commit` between tasks.
fn score(
    worktree: &Path,
    commit: &str,
    tasks: &[HeldOutTask],
    runner: &dyn EngineRunner,
    idle: Duration,
) -> (Score, u64, f64) {
    let mut s = Score {
        total: tasks.len(),
        ..Default::default()
    };
    let (mut tokens, mut cost) = (0u64, 0.0f64);
    for t in tasks {
        reset_worktree(worktree, commit);
        let run = agent_run::run_with_runner(
            &RunSpec {
                engine: Engine::Claude,
                cwd: worktree,
                prompt: &t.prompt,
                policy: ToolPolicy::AcceptEdits,
                model: None,
                resume: None,
                idle_timeout: idle,
                on_activity: None,
            },
            runner,
        );
        let mut excerpt = String::new();
        if let Ok(o) = &run {
            tokens = tokens.saturating_add(o.tokens);
            cost += o.cost_usd;
        } else if let Err(e) = &run {
            excerpt = format!("run failed: {e}\n");
        }
        let (ok, out) = run_check(worktree, &t.check);
        excerpt.push_str(&out);
        if ok {
            s.passed += 1;
        }
        s.tasks.push(TaskResult {
            id: t.id.clone(),
            passed: ok,
            excerpt,
        });
    }
    reset_worktree(worktree, commit);
    (s, tokens, cost)
}

fn edit_prompt(cfg: &LabConfig, parent: Option<&Experiment>) -> String {
    let lineage = match parent {
        Some(p) => format!(
            "The previous kept experiment ({}) changed {} and its hypothesis was: {}",
            p.id,
            p.files_changed.join(", "),
            p.hypothesis
        ),
        None => "There is no previous kept experiment.".into(),
    };
    format!(
        "You are improving this project's AGENT HARNESS — the instructions and skills a coding agent \
reads — not its source code. The metric is `{metric}`: the pass rate of a hidden set of tasks run by a \
fresh agent under this harness, each checked by a command you cannot see. You may edit ONLY these paths: \
{editable}. Any change outside them is rejected automatically. Do not create tests, do not edit the evaluator, \
do not run long commands.\n\n{focus}\n{lineage}\n\n\
Make ONE focused change that you expect to raise the metric (clearer conventions, a missing command, a \
sharper skill, a guardrail that prevents a common mistake). Then reply with exactly one line:\n\
HYPOTHESIS: <one sentence on what you changed and why it should help>",
        metric = cfg.metric,
        editable = cfg.editable.join(", "),
        focus = if cfg.focus.trim().is_empty() {
            String::new()
        } else {
            format!("Focus from the human: {}\n", cfg.focus.trim())
        },
        lineage = lineage,
    )
}

fn hypothesis_from(text: &str) -> String {
    text.lines()
        .rev()
        .find_map(|l| {
            l.trim()
                .strip_prefix("HYPOTHESIS:")
                .map(|h| h.trim().to_string())
        })
        .unwrap_or_else(|| text.trim().chars().take(240).collect())
}

fn witness(testigo: &TestigoService, project_root: &str, ts: i64, kind: &str, payload: Value) {
    let _ = testigo.on_harness(project_root, ts, kind, payload);
}

/// One experiment, end to end, with the engine adapters injected (tests use
/// fakes; the scheduler and the command pass the real Claude runner twice).
#[allow(clippy::too_many_arguments)]
pub fn run_experiment_with(
    testigo: &TestigoService,
    project_root: &str,
    editor: &dyn EngineRunner,
    solver: &dyn EngineRunner,
    now_ms: i64,
) -> AppResult<Experiment> {
    let mut st = load(project_root)?;
    if st.heldout.tasks.is_empty() {
        return Err(AppError::Other(
            "Declare held-out tasks first (Learning → Harness lab): the metric needs them".into(),
        ));
    }
    let repo = Path::new(project_root);
    let n = st.experiments.len();
    let id = format!("hx-{}-{:03}", now_ms / 1000, n + 1);
    let parent = st
        .experiments
        .iter()
        .rev()
        .find(|e| e.status == "kept")
        .cloned();
    let selected = select_tasks(&st.heldout.tasks, n);
    let selected_ids: Vec<String> = selected.iter().map(|t| t.id.clone()).collect();
    let idle = Duration::from_secs(st.config.budget.idle_minutes * 60);
    let branch = format!("harness/{id}");
    let wt_dir = lab_dir(project_root)?.join("worktrees").join(&id);

    let mut exp = Experiment {
        id: id.clone(),
        parent_id: parent.as_ref().map(|p| p.id.clone()),
        created_ms: now_ms,
        finished_ms: None,
        metric: st.config.metric.clone(),
        hypothesis: String::new(),
        branch: branch.clone(),
        status: "running".into(),
        stop_reason: None,
        heldout_version: st.heldout.version,
        selected_tasks: selected_ids.clone(),
        baseline: None,
        score: None,
        files_changed: Vec::new(),
        outside_files: Vec::new(),
        tokens: 0,
        cost_usd: 0.0,
        pr_url: None,
        commit: None,
    };
    st.experiments.push(exp.clone());
    save(project_root, &st)?;
    witness(
        testigo,
        project_root,
        now_ms,
        "experiment_started",
        json!({ "id": id, "parent": exp.parent_id, "metric": exp.metric, "branch": branch, "heldoutVersion": exp.heldout_version, "tasks": selected_ids.len(), "budget": st.config.budget }),
    );

    let finish = |mut exp: Experiment, st: &mut LabState| -> AppResult<Experiment> {
        exp.finished_ms = Some(now_ms);
        if let Some(slot) = st.experiments.iter_mut().find(|e| e.id == exp.id) {
            *slot = exp.clone();
        }
        save(project_root, st)?;
        witness(
            testigo,
            project_root,
            now_ms,
            "experiment_result",
            json!({
                "id": exp.id, "parent": exp.parent_id, "status": exp.status, "stopReason": exp.stop_reason,
                "hypothesis": exp.hypothesis, "branch": exp.branch, "commit": exp.commit,
                "baseline": exp.baseline.as_ref().map(|s| json!({ "passed": s.passed, "total": s.total })),
                "score": exp.score.as_ref().map(|s| json!({ "passed": s.passed, "total": s.total })),
                "filesChanged": exp.files_changed, "outsideFiles": exp.outside_files,
                "tokens": exp.tokens, "costUsd": exp.cost_usd, "prUrl": exp.pr_url,
            }),
        );
        Ok(exp)
    };

    // The worktree: the sandbox the agent gets, never the checkout itself.
    let wt = match worktree_service::create(repo, &wt_dir, &branch, None) {
        Ok(w) => w,
        Err(e) => {
            exp.status = "failed".into();
            exp.stop_reason = Some(format!("worktree: {e}"));
            return finish(exp, &mut st);
        }
    };
    let wt_path = PathBuf::from(&wt.path);
    let base_commit = String::from_utf8_lossy(&git(&wt_path, &["rev-parse", "HEAD"])?.stdout)
        .trim()
        .to_string();

    // Baseline on the unmodified harness (cached per held-out version + window).
    let key = format!("{}:{}", st.heldout.version, selected_ids.join(","));
    let baseline = match st.baselines.get(&key) {
        Some(b) => b.clone(),
        None => {
            let (b, t, c) = score(&wt_path, &base_commit, &selected, solver, idle);
            exp.tokens += t;
            exp.cost_usd += c;
            st.baselines.insert(key.clone(), b.clone());
            b
        }
    };
    exp.baseline = Some(baseline.clone());

    // The edit, under budget.
    let prompt = edit_prompt(&st.config, parent.as_ref());
    let edit = agent_run::run_with_runner(
        &RunSpec {
            engine: Engine::Claude,
            cwd: &wt_path,
            prompt: &prompt,
            policy: ToolPolicy::AcceptEdits,
            model: None,
            resume: None,
            idle_timeout: idle,
            on_activity: None,
        },
        editor,
    );
    let edit = match edit {
        Ok(o) => o,
        Err(e) => {
            exp.status = "failed".into();
            exp.stop_reason = Some(format!("edit run: {e}"));
            let _ = worktree_service::discard(repo, &wt_path, &branch, true);
            return finish(exp, &mut st);
        }
    };
    exp.tokens += edit.tokens;
    exp.cost_usd += edit.cost_usd;
    exp.hypothesis = hypothesis_from(&edit.text);

    // The gate the agent cannot argue with: what did it actually touch?
    let changed = changed_files(&wt_path)?;
    exp.files_changed = changed.clone();
    exp.outside_files = changed
        .iter()
        .filter(|p| !is_editable(p, &st.config.editable))
        .cloned()
        .collect();
    if !exp.outside_files.is_empty() {
        exp.status = "rejected".into();
        exp.stop_reason = Some(format!(
            "touched outside the harness: {}",
            exp.outside_files.join(", ")
        ));
        let _ = worktree_service::discard(repo, &wt_path, &branch, true);
        return finish(exp, &mut st);
    }
    if changed.is_empty() {
        exp.status = "reverted".into();
        exp.stop_reason = Some("no change proposed".into());
        let _ = worktree_service::discard(repo, &wt_path, &branch, true);
        return finish(exp, &mut st);
    }
    if exp.tokens > st.config.budget.max_tokens {
        exp.status = "reverted".into();
        exp.stop_reason = Some(format!(
            "over budget: {} tokens > {}",
            exp.tokens, st.config.budget.max_tokens
        ));
        let _ = worktree_service::discard(repo, &wt_path, &branch, true);
        return finish(exp, &mut st);
    }

    // Commit the harness change with its lineage, then score under it.
    let mut add = vec!["add", "--"];
    let changed_refs: Vec<&str> = changed.iter().map(String::as_str).collect();
    add.extend(changed_refs);
    let _ = git(&wt_path, &add);
    let msg = format!(
        "harness: {}\n\nHarness-Experiment: {}\nHarness-Parent: {}\nHarness-Metric: {}\n",
        exp.hypothesis.chars().take(72).collect::<String>(),
        id,
        exp.parent_id.clone().unwrap_or_else(|| "none".into()),
        exp.metric
    );
    let commit_out = git(&wt_path, &["commit", "-q", "-m", &msg])?;
    if !commit_out.status.success() {
        exp.status = "failed".into();
        exp.stop_reason = Some(format!(
            "commit: {}",
            String::from_utf8_lossy(&commit_out.stderr).trim()
        ));
        let _ = worktree_service::discard(repo, &wt_path, &branch, true);
        return finish(exp, &mut st);
    }
    let harness_commit = String::from_utf8_lossy(&git(&wt_path, &["rev-parse", "HEAD"])?.stdout)
        .trim()
        .to_string();
    exp.commit = Some(harness_commit.clone());
    let (sc, t, c) = score(&wt_path, &harness_commit, &selected, solver, idle);
    exp.tokens += t;
    exp.cost_usd += c;
    exp.score = Some(sc.clone());

    if sc.passed > baseline.passed {
        exp.status = "kept".into();
        // Push and offer a PR when there is a remote; a kept branch stays
        // either way. Never merged from here.
        match crate::services::git_service::push_current(&wt_path) {
            Ok(p) => exp.pr_url = p.pr_url,
            Err(e) => exp.stop_reason = Some(format!("kept locally (push: {e})")),
        }
    } else {
        exp.status = "reverted".into();
        exp.stop_reason = Some(format!(
            "no improvement: {}/{} vs baseline {}/{}",
            sc.passed, sc.total, baseline.passed, baseline.total
        ));
        let _ = worktree_service::discard(repo, &wt_path, &branch, true);
    }
    finish(exp, &mut st)
}

/// With the real adapter: the same frozen model edits the harness and then
/// solves the hidden tasks under it.
pub fn run_experiment(
    testigo: &TestigoService,
    project_root: &str,
    now_ms: i64,
) -> AppResult<Experiment> {
    let r = crate::services::engine_runner::runner_for(Engine::Claude);
    run_experiment_with(testigo, project_root, r, r, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::engine_runner::{RunCtx, TurnOutput};

    #[test]
    fn editable_is_the_harness_not_the_code() {
        let e: Vec<String> = EDITABLE.iter().map(|s| s.to_string()).collect();
        assert!(is_editable("CLAUDE.md", &e));
        assert!(is_editable(".claude/skills/foo/SKILL.md", &e));
        assert!(!is_editable("src/main.rs", &e));
        assert!(!is_editable("tests/x.test.ts", &e));
        assert!(!is_editable(".claude/settings.local.json", &e));
        assert!(!is_editable("CLAUDE.md.bak", &e));
    }

    #[test]
    fn the_window_rotates_through_every_task() {
        let tasks: Vec<HeldOutTask> = (0..5)
            .map(|i| HeldOutTask {
                id: format!("t{i}"),
                prompt: "p".into(),
                check: "true".into(),
            })
            .collect();
        let ids = |n: usize| {
            select_tasks(&tasks, n)
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(0), vec!["t0", "t1", "t2"]);
        assert_eq!(ids(3), vec!["t3", "t4", "t0"]);
        assert_eq!(select_tasks(&tasks[..2], 7).len(), 2);
        assert!(select_tasks(&[], 0).is_empty());
        assert_eq!(
            hypothesis_from("blah\nHYPOTHESIS: shorter CLAUDE.md helps\n"),
            "shorter CLAUDE.md helps"
        );
    }

    /// A fake engine: the editor writes what the test tells it to; the solver
    /// does nothing (the checks read the harness file directly).
    struct Fake {
        write: Option<(&'static str, &'static str)>,
    }
    impl EngineRunner for Fake {
        fn run(
            &self,
            ctx: &RunCtx,
            _on: &crate::services::engine_runner::ActivitySink,
        ) -> AppResult<TurnOutput> {
            if ctx.prompt.contains("AGENT HARNESS") {
                if let Some((path, body)) = self.write {
                    let p = ctx.cwd.join(path);
                    if let Some(parent) = p.parent() {
                        fs::create_dir_all(parent).unwrap();
                    }
                    fs::write(p, body).unwrap();
                }
                return Ok(TurnOutput {
                    text: "done\nHYPOTHESIS: say what to run".into(),
                    session_id: None,
                    tokens: 100,
                    cost_usd: 0.01,
                });
            }
            Ok(TurnOutput {
                text: "ok".into(),
                session_id: None,
                tokens: 10,
                cost_usd: 0.0,
            })
        }
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p =
                std::env::temp_dir().join(format!("ac-lab-{tag}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn repo_with_harness() -> (TempDir, String) {
        let dir = TempDir::new("repo");
        let root = dir.path().to_string_lossy().to_string();
        let g = |args: &[&str]| {
            let o = proc::command("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{:?}: {}",
                args,
                String::from_utf8_lossy(&o.stderr)
            );
        };
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        fs::write(dir.path().join("CLAUDE.md"), "# project\n").unwrap();
        fs::write(dir.path().join("evaluator.txt"), "do not touch\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-q", "-m", "init"]);
        (dir, root)
    }

    fn with_lab_env<T>(f: impl FnOnce() -> T) -> T {
        let _env = crate::test_support::lock_env();
        let base = TempDir::new("data");
        std::env::set_var("XDG_DATA_HOME", base.path());
        f()
    }

    #[test]
    fn a_harness_change_that_raises_the_hidden_score_is_kept_with_lineage() {
        with_lab_env(|| {
            let (_dir, root) = repo_with_harness();
            set_heldout(
                &root,
                vec![HeldOutTask {
                    id: "t1".into(),
                    prompt: "do the task".into(),
                    check: "grep -q 'npm test' CLAUDE.md".into(),
                }],
            )
            .unwrap();
            let testigo = TestigoService::new();
            let editor = Fake {
                write: Some((
                    "CLAUDE.md",
                    "# project\nRun `npm test` before every commit.\n",
                )),
            };
            let solver = Fake { write: None };
            let exp = run_experiment_with(&testigo, &root, &editor, &solver, 1_000).unwrap();
            assert_eq!(exp.status, "kept", "{:?}", exp.stop_reason);
            assert_eq!(exp.baseline.as_ref().unwrap().passed, 0);
            assert_eq!(exp.score.as_ref().unwrap().passed, 1);
            assert_eq!(exp.files_changed, vec!["CLAUDE.md"]);
            assert!(exp.outside_files.is_empty());
            assert_eq!(exp.hypothesis, "say what to run");
            assert!(exp.commit.is_some());
            assert!(exp.pr_url.is_none(), "no remote in the test repo");
            // The main checkout is untouched: the change lives on the branch.
            assert_eq!(
                fs::read_to_string(Path::new(&root).join("CLAUDE.md")).unwrap(),
                "# project\n"
            );
            let log = proc::command("git")
                .args(["log", "-1", "--format=%B", &exp.branch])
                .current_dir(&root)
                .output()
                .unwrap();
            let body = String::from_utf8_lossy(&log.stdout);
            assert!(body.contains(&format!("Harness-Experiment: {}", exp.id)));
            assert!(body.contains("Harness-Parent: none"));
            let s = summary(&root).unwrap();
            assert_eq!(s.kept, 1);
            assert_eq!(s.clean_runs, 1);
            let kinds: Vec<String> = testigo
                .list(&root, None, None)
                .unwrap()
                .iter()
                .map(|e| e.kind.clone())
                .collect();
            assert!(kinds.contains(&"experiment_started".into()));
            assert!(kinds.contains(&"experiment_result".into()));

            // A second experiment names the first as its parent in the trailer.
            let exp2 = run_experiment_with(&testigo, &root, &Fake { write: None }, &solver, 2_000)
                .unwrap();
            assert_eq!(exp2.parent_id.as_deref(), Some(exp.id.as_str()));
            assert_eq!(exp2.status, "reverted");
            assert_eq!(exp2.stop_reason.as_deref(), Some("no change proposed"));
        });
    }

    #[test]
    fn touching_the_evaluator_is_rejected_after_the_fact() {
        with_lab_env(|| {
            let (_dir, root) = repo_with_harness();
            set_heldout(
                &root,
                vec![HeldOutTask {
                    id: "t1".into(),
                    prompt: "x".into(),
                    check: "true".into(),
                }],
            )
            .unwrap();
            let testigo = TestigoService::new();
            let editor = Fake {
                write: Some(("evaluator.txt", "hacked\n")),
            };
            let exp = run_experiment_with(&testigo, &root, &editor, &Fake { write: None }, 1_000)
                .unwrap();
            assert_eq!(exp.status, "rejected");
            assert_eq!(exp.outside_files, vec!["evaluator.txt"]);
            assert_eq!(summary(&root).unwrap().clean_runs, 0);
            // Branch and worktree are gone; the checkout is clean.
            let b = proc::command("git")
                .args(["branch", "--list", &exp.branch])
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(String::from_utf8_lossy(&b.stdout).trim().is_empty());
            assert_eq!(
                fs::read_to_string(Path::new(&root).join("evaluator.txt")).unwrap(),
                "do not touch\n"
            );
        });
    }
}
