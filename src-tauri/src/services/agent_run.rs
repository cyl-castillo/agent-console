//! One way to run an agent turn headlessly.
//!
//! Advisor, reflect, curator, scheduler jobs and rooms all spawn
//! `claude -p` / `codex exec`; until now they were five near-copies with
//! different policies, four of them without any timeout. This module is the
//! single entry: a [`RunSpec`] in, a [`RunOutcome`] out (final text plus the
//! engine's own session id, token count and dollar cost), on top of the
//! engine adapters in `engine_runner`. It adds what every unattended run
//! needs and none had:
//!
//! - an idle watchdog: a turn that emits nothing for `idle_timeout` is hung
//!   (network stall, wedged CLI) and gets killed instead of holding a thread
//!   and a "running" state forever;
//! - the unknown-model recovery: when the user's configured model predates
//!   the installed CLI, retry once with the family alias (see
//!   `claude_cli::unknown_model_in`).
//!
//! Rooms keep their own driver (they interleave turns, pause and inject) but
//! use the same adapters and the same slot/kill mechanics.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::services::claude_cli;
use crate::services::engine_runner::{
    self, ActivitySink, ChildSlot, Engine, EngineRunner, RunCtx, ToolPolicy,
};

/// Default silence an unattended turn may keep before it is killed. Long
/// tool runs emit nothing until they finish, so this is generous.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const WATCHDOG_TICK: Duration = Duration::from_millis(250);

/// `AGENT_CONSOLE_HEADLESS_IDLE_SECS` overrides the default; anything that
/// isn't a positive integer keeps it.
pub fn default_idle_timeout() -> Duration {
    idle_timeout_from(
        std::env::var("AGENT_CONSOLE_HEADLESS_IDLE_SECS")
            .ok()
            .as_deref(),
    )
}

pub fn idle_timeout_from(raw: Option<&str>) -> Duration {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_IDLE_TIMEOUT)
}

pub struct RunSpec<'a> {
    pub engine: Engine,
    pub cwd: &'a Path,
    pub prompt: &'a str,
    pub policy: ToolPolicy,
    /// Claude: model alias/id; Codex: reasoning effort. `None` = the user's
    /// configured default — what unattended runs should use.
    pub model: Option<&'a str>,
    /// Resume id from a prior turn of the same participant.
    pub resume: Option<&'a str>,
    pub idle_timeout: Duration,
    /// Live activity (thinking / tool / text), when the caller shows it.
    pub on_activity: Option<&'a ActivitySink<'a>>,
}

#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub text: String,
    pub session_id: Option<String>,
    /// Real new tokens (cache reads excluded), as the engine reports them.
    pub tokens: u64,
    /// Dollar cost as reported by the CLI (Codex reports none ⇒ 0.0).
    pub cost_usd: f64,
    /// The family alias the unknown-model retry fell back to, when it did.
    pub model_fallback: Option<&'static str>,
}

/// Run one headless turn with the engine's adapter.
pub fn run(spec: &RunSpec) -> AppResult<RunOutcome> {
    run_with_runner(spec, engine_runner::runner_for(spec.engine))
}

/// Same, with the adapter injected (tests use a fake that parks a child).
pub fn run_with_runner(spec: &RunSpec, runner: &dyn EngineRunner) -> AppResult<RunOutcome> {
    match attempt(spec, runner, spec.model.unwrap_or("")) {
        Ok(o) => Ok(o),
        Err(e) => {
            // The configured model predates the installed CLI: retry once on
            // the family alias every version resolves. Only when the caller
            // left the model to the user's default — a model the caller
            // chose is the caller's to fix.
            let Some(alias) = retry_alias_for(spec.engine, spec.model, &e.to_string()) else {
                return Err(e);
            };
            tracing::warn!(
                "agent_run: configured model unknown to this CLI; retrying with {alias}"
            );
            let mut o = attempt(spec, runner, alias)?;
            o.model_fallback = Some(alias);
            Ok(o)
        }
    }
}

/// Which family alias to retry with, if this failure is the unknown-model
/// one, the engine is Claude, and the caller did not pick the model.
pub fn retry_alias_for(engine: Engine, chosen: Option<&str>, error: &str) -> Option<&'static str> {
    if engine != Engine::Claude || chosen.is_some_and(|m| !m.is_empty()) {
        return None;
    }
    claude_cli::unknown_model_in(error).and_then(|id| claude_cli::family_alias(&id))
}

fn attempt(spec: &RunSpec, runner: &dyn EngineRunner, model: &str) -> AppResult<RunOutcome> {
    let slot = Arc::new(ChildSlot::default());
    let last_activity = Arc::new(AtomicU64::new(now_ms()));
    let done = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    {
        let (slot, last, done, timed_out) = (
            slot.clone(),
            last_activity.clone(),
            done.clone(),
            timed_out.clone(),
        );
        let limit = spec.idle_timeout.as_millis() as u64;
        std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(WATCHDOG_TICK);
                if now_ms().saturating_sub(last.load(Ordering::SeqCst)) > limit {
                    timed_out.store(true, Ordering::SeqCst);
                    engine_runner::kill_parked(&slot);
                    return;
                }
            }
        });
    }
    let stamp = last_activity.clone();
    let sink = |kind: &str, label: &str, text: &str| {
        stamp.store(now_ms(), Ordering::SeqCst);
        if let Some(cb) = spec.on_activity {
            cb(kind, label, text);
        }
    };
    let ctx = RunCtx {
        cwd: spec.cwd,
        model,
        tools: spec.policy,
        prompt: spec.prompt,
        resume: spec.resume,
        child_slot: Some(&slot),
    };
    let result = runner.run(&ctx, &sink);
    done.store(true, Ordering::SeqCst);
    match result {
        Ok(t) => Ok(RunOutcome {
            text: t.text,
            session_id: t.session_id,
            tokens: t.tokens,
            cost_usd: t.cost_usd,
            model_fallback: None,
        }),
        Err(e) if timed_out.load(Ordering::SeqCst) => Err(AppError::Other(format!(
            "agent produced no output for {} s and was killed (AGENT_CONSOLE_HEADLESS_IDLE_SECS to change): {e}",
            spec.idle_timeout.as_secs()
        ))),
        Err(e) => Err(e),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_timeout_env_override_only_accepts_positive_seconds() {
        assert_eq!(idle_timeout_from(None), DEFAULT_IDLE_TIMEOUT);
        assert_eq!(idle_timeout_from(Some("0")), DEFAULT_IDLE_TIMEOUT);
        assert_eq!(idle_timeout_from(Some("x")), DEFAULT_IDLE_TIMEOUT);
        assert_eq!(idle_timeout_from(Some(" 45 ")), Duration::from_secs(45));
    }

    #[test]
    fn retry_only_for_claude_on_the_users_default_model() {
        let err = "claude exited with status 1: \"claude-opus-5-5\" isn't described by this version's model catalog";
        assert_eq!(retry_alias_for(Engine::Claude, None, err), Some("opus"));
        assert_eq!(retry_alias_for(Engine::Claude, Some(""), err), Some("opus"));
        // The caller chose a model: theirs to fix, no silent swap.
        assert_eq!(retry_alias_for(Engine::Claude, Some("fable"), err), None);
        // Codex has no such catalog error.
        assert_eq!(retry_alias_for(Engine::Codex, None, err), None);
        // Any other failure: no retry.
        assert_eq!(retry_alias_for(Engine::Claude, None, "Not logged in"), None);
    }

    /// A runner that parks a real child (`sleep`) in the slot and then waits
    /// on it — exactly what the adapters do — emitting no activity, so the
    /// watchdog must kill it.
    #[cfg(unix)]
    struct HangingRunner;
    #[cfg(unix)]
    impl EngineRunner for HangingRunner {
        fn run(
            &self,
            ctx: &RunCtx,
            _on_activity: &ActivitySink,
        ) -> AppResult<engine_runner::TurnOutput> {
            let child = std::process::Command::new("sleep")
                .arg("30")
                .stdout(std::process::Stdio::null())
                .spawn()
                .map_err(|e| AppError::Other(e.to_string()))?;
            let slot = ctx.child_slot.expect("agent_run always passes a slot");
            *slot.lock() = Some(child);
            // Keep it parked while it runs (the real adapters read stdout
            // here); take it back only to reap it, like they do.
            loop {
                std::thread::sleep(Duration::from_millis(50));
                let mut g = slot.lock();
                if let Some(c) = g.as_mut() {
                    if let Ok(Some(_)) = c.try_wait() {
                        break;
                    }
                } else {
                    break;
                }
            }
            let mut child = slot.lock().take().expect("nobody else takes it");
            let status = child.wait().map_err(|e| AppError::Other(e.to_string()))?;
            if status.success() {
                Ok(engine_runner::TurnOutput {
                    text: "done".into(),
                    session_id: None,
                    tokens: 0,
                    cost_usd: 0.0,
                })
            } else {
                Err(AppError::Other(format!("child exited {status}")))
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn watchdog_kills_a_turn_that_goes_quiet() {
        let spec = RunSpec {
            engine: Engine::Claude,
            cwd: std::path::Path::new("."),
            prompt: "p",
            policy: ToolPolicy::Plan,
            model: None,
            resume: None,
            idle_timeout: Duration::from_millis(400),
            on_activity: None,
        };
        let started = std::time::Instant::now();
        let err = run_with_runner(&spec, &HangingRunner)
            .unwrap_err()
            .to_string();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "killed, not waited out"
        );
        assert!(err.contains("no output for"), "{err}");
        assert!(err.contains("HEADLESS_IDLE_SECS"), "{err}");
    }
}
