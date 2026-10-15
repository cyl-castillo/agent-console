//! D10: a reviewer agent of its own. A second agent — a different identity,
//! a different engine or model whenever one is installed — reads what a
//! turn changed and argues against it ("adversarial verification"). Its
//! verdict is evidence: `review_verdict` in the ledger under
//! `agent:reviewer:<engine>`, a decision card in the Inbox, and a gate the
//! proof-on-PR check honours: a `block` stands until a human records a
//! `review_decision` that overrides it.
//!
//! Two layers, on purpose. Deterministic rules the model cannot talk its way
//! around (a protected test edited together with the code it covers; a
//! summary that claims tests passed with no check run witnessed) decide the
//! floor of the verdict. The model adds what rules cannot see — intent
//! drift, a fix that silences the symptom — and can only make the verdict
//! stricter, never looser.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::agent_run::{self, RunSpec};
use crate::services::engine_runner::{Engine, EngineRunner, ToolPolicy};
use crate::services::testigo_service::{ProofEvent, TestigoService};
use crate::services::{claude_cli, proc};

/// Hard cap on the diff the reviewer reads: enough for a real turn, bounded
/// so a generated-file blast cannot blow the prompt.
const DIFF_MAX_CHARS: usize = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Approve,
    RequestChanges,
    Block,
}

impl Verdict {
    fn rank(self) -> u8 {
        match self {
            Verdict::Approve => 0,
            Verdict::RequestChanges => 1,
            Verdict::Block => 2,
        }
    }
    fn stricter(self, other: Verdict) -> Verdict {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
    fn parse(s: &str) -> Option<Verdict> {
        match s.trim().to_ascii_lowercase().as_str() {
            "approve" | "approved" | "ok" | "lgtm" => Some(Verdict::Approve),
            "request_changes" | "changes" | "request changes" => Some(Verdict::RequestChanges),
            "block" | "blocked" | "reject" => Some(Verdict::Block),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// block | high | medium | low
    pub severity: String,
    /// Stable rule id for deterministic findings (`test_edited`,
    /// `unverified_claim`); free text for the model's.
    pub rule: String,
    pub title: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// True when a rule, not the model, produced it.
    pub deterministic: bool,
}

/// What the reviewer was given: the turn as the ledger holds it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewScope {
    pub term_id: String,
    pub turn_id: String,
    pub case_id: String,
    pub prompt: String,
    pub summary: String,
    pub pre_sha: Option<String>,
    pub post_sha: Option<String>,
    /// (status, path) as turn_end.filesChanged.
    pub files: Vec<(String, String)>,
    pub checks_passed: usize,
    pub checks_failed: usize,
    pub producer_model: Option<String>,
    pub producer_engine: Engine,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewVerdict {
    pub verdict: Verdict,
    pub findings: Vec<Finding>,
    pub reviewer: String,
    pub reviewer_engine: Engine,
    pub reviewer_model: Option<String>,
    pub producer_model: Option<String>,
    pub term_id: String,
    pub turn_id: String,
    pub case_id: String,
    pub files: usize,
    pub tokens: u64,
    pub cost_usd: f64,
    /// Verbatim model answer (capped) — the human can read the argument.
    pub rationale: String,
    pub seq: Option<u64>,
}

fn looks_like_test(path: &str) -> bool {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    p.starts_with("tests/")
        || p.starts_with("test/")
        || p.contains("/tests/")
        || p.contains("/test/")
        || p.contains("/__tests__/")
        || p.starts_with("__tests__/")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.ends_with("_test.go")
        || name.ends_with("_test.py")
        || name.starts_with("test_") && name.ends_with(".py")
        || name.ends_with("_spec.rb")
}

fn asked_for_tests(prompt: &str) -> bool {
    let p = prompt.to_ascii_lowercase();
    ["test", "spec", "prueba"].iter().any(|w| p.contains(w))
}

/// The floor of the verdict: rules the model cannot argue with.
pub fn deterministic_findings(scope: &ReviewScope) -> Vec<Finding> {
    let mut out = Vec::new();
    let tests: Vec<&str> = scope
        .files
        .iter()
        .filter(|(_, p)| looks_like_test(p))
        .map(|(_, p)| p.as_str())
        .collect();
    let sources = scope
        .files
        .iter()
        .filter(|(_, p)| !looks_like_test(p))
        .count();
    if !tests.is_empty() && sources > 0 {
        let asked = asked_for_tests(&scope.prompt);
        out.push(Finding {
            severity: if asked { "high".into() } else { "block".into() },
            rule: "test_edited".into(),
            title: format!(
                "{} test file{} changed together with {} source file{}",
                tests.len(),
                if tests.len() == 1 { "" } else { "s" },
                sources,
                if sources == 1 { "" } else { "s" }
            ),
            detail: if asked {
                "The prompt asked for test work, so this is expected — still verify the assertions were not weakened to make the change pass.".into()
            } else {
                "The prompt did not ask for test changes. A test edited alongside the code it covers is how an agent makes a failing change look green; a human has to read the assertion before this merges.".into()
            },
            file: tests.first().map(|s| s.to_string()),
            deterministic: true,
        });
    }
    let claims_green = {
        let s = scope.summary.to_ascii_lowercase();
        [
            "tests pass",
            "all tests pass",
            "test passes",
            "passing",
            "green",
            "tests are passing",
        ]
        .iter()
        .any(|w| s.contains(w))
    };
    if claims_green && scope.checks_passed == 0 {
        out.push(Finding {
            severity: "high".into(),
            rule: "unverified_claim".into(),
            title: "The summary claims tests pass, but no passing check run was witnessed".into(),
            detail: "No check_run event with status passed exists in this turn. Either the tests never ran in a way the console could see, or the claim is unfounded.".into(),
            file: None,
            deterministic: true,
        });
    }
    out
}

fn floor_from(findings: &[Finding]) -> Verdict {
    let mut v = Verdict::Approve;
    for f in findings {
        v = v.stricter(match f.severity.as_str() {
            "block" => Verdict::Block,
            "high" => Verdict::RequestChanges,
            _ => Verdict::Approve,
        });
    }
    v
}

/// A different identity from the producer: the other engine when it is
/// installed, else the other model family of the same engine.
pub fn pick_reviewer(
    producer_engine: Engine,
    producer_model: Option<&str>,
    codex_available: bool,
    claude_available: bool,
) -> (Engine, Option<String>) {
    match producer_engine {
        Engine::Claude if codex_available => (Engine::Codex, None),
        Engine::Codex if claude_available => (Engine::Claude, None),
        Engine::Claude => {
            let alias = producer_model.and_then(claude_cli::family_alias);
            let other = match alias {
                Some("opus") | Some("fable") => "sonnet",
                Some("sonnet") | Some("haiku") => "opus",
                _ => "sonnet",
            };
            (Engine::Claude, Some(other.to_string()))
        }
        Engine::Codex => (Engine::Codex, Some("high".to_string())),
    }
}

fn engine_available(bin: &str) -> bool {
    proc::command(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Assemble the turn from the ledger: its prompt, close, checks and model.
pub fn scope_for(
    testigo: &TestigoService,
    project_root: &str,
    term_id: &str,
    turn_id: Option<&str>,
) -> AppResult<ReviewScope> {
    let events = testigo.list(project_root, None, None)?;
    let mine: Vec<&ProofEvent> = events
        .iter()
        .filter(|e| e.term_id.as_deref() == Some(term_id))
        .collect();
    // The turn asked for, else the last closed turn of the session.
    let turn_id = match turn_id {
        Some(t) => t.to_string(),
        None => mine
            .iter()
            .rev()
            .find(|e| e.kind == "turn_end")
            .and_then(|e| e.turn_id.clone())
            .ok_or_else(|| AppError::Other("No finished turn to review in this session".into()))?,
    };
    let mut scope = ReviewScope {
        term_id: term_id.to_string(),
        turn_id: turn_id.clone(),
        ..Default::default()
    };
    let mut model: Option<String> = None;
    for e in &mine {
        let p = &e.payload;
        if e.kind == "session_start" {
            if let Some(m) = p.get("model").and_then(Value::as_str) {
                model = Some(m.to_string());
            }
            if let Some(a) = p.get("agent").and_then(Value::as_str) {
                scope.producer_engine = if a == "codex" {
                    Engine::Codex
                } else {
                    Engine::Claude
                };
            }
        }
        if e.kind == "model_switch" {
            if let Some(m) = p.get("to").and_then(Value::as_str) {
                model = Some(m.to_string());
            }
        }
        if e.turn_id.as_deref() != Some(&turn_id) {
            continue;
        }
        match e.kind.as_str() {
            "prompt" => {
                scope.case_id = e.case_id.clone();
                scope.prompt = p
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
            }
            "check_run" => {
                if p.get("status").and_then(Value::as_str) == Some("passed") {
                    scope.checks_passed += 1;
                } else {
                    scope.checks_failed += 1;
                }
            }
            "turn_end" => {
                scope.summary = p
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                scope.pre_sha = p.get("preSha").and_then(Value::as_str).map(String::from);
                scope.post_sha = p.get("postSha").and_then(Value::as_str).map(String::from);
                if let Some(files) = p.get("filesChanged").and_then(Value::as_array) {
                    scope.files = files
                        .iter()
                        .filter_map(|f| {
                            Some((
                                f.get("status")?.as_str()?.to_string(),
                                f.get("path")?.as_str()?.to_string(),
                            ))
                        })
                        .collect();
                }
            }
            _ => {}
        }
    }
    if scope.case_id.is_empty() {
        scope.case_id = mine
            .iter()
            .find(|e| e.turn_id.as_deref() == Some(&turn_id))
            .map(|e| e.case_id.clone())
            .ok_or_else(|| AppError::Other(format!("Turn {turn_id} is not in the ledger")))?;
    }
    if model.is_some() && scope.producer_model.is_none() {
        scope.producer_model = model;
    }
    Ok(scope)
}

/// The turn's diff between its snapshots (capped). Empty when the turn has
/// no snapshots or git refuses — the rules still run on the file list.
pub fn diff_text(repo: &Path, scope: &ReviewScope) -> String {
    let (Some(pre), Some(post)) = (&scope.pre_sha, &scope.post_sha) else {
        return String::new();
    };
    let out = proc::command("git")
        .args(["diff", "--no-color", "--stat=120", "--patch", pre, post])
        .current_dir(repo)
        .output();
    let Ok(out) = out else { return String::new() };
    if !out.status.success() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.chars().count() > DIFF_MAX_CHARS {
        let cut: String = text.chars().take(DIFF_MAX_CHARS).collect();
        format!("{cut}\n… diff truncated at {DIFF_MAX_CHARS} characters …")
    } else {
        text
    }
}

pub fn build_prompt(scope: &ReviewScope, diff: &str, floor: &[Finding]) -> String {
    let files = scope
        .files
        .iter()
        .map(|(s, p)| format!("{s} {p}"))
        .collect::<Vec<_>>()
        .join("\n");
    let floor_text = if floor.is_empty() {
        "none".to_string()
    } else {
        floor
            .iter()
            .map(|f| format!("- [{}] {}: {}", f.severity, f.rule, f.title))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "You are an adversarial code reviewer. Another agent (not you) produced the change below in one turn. \
Your job is to find what is wrong, unverified, or not what was asked — not to be agreeable. \
Assume the producer may have made the symptom disappear instead of fixing the cause, weakened a test, \
or claimed a verification it never ran. Do not run anything; read only.\n\n\
## What the human asked\n{prompt}\n\n\
## What the producer said when it finished\n{summary}\n\n\
## Witnessed check runs in this turn\n{passed} passed, {failed} failed\n\n\
## Files changed\n{files}\n\n\
## Rule findings already established (you cannot lower these)\n{floor}\n\n\
## Diff\n```diff\n{diff}\n```\n\n\
Answer with strict JSON only, no prose before or after:\n\
{{\"verdict\": \"approve\" | \"request_changes\" | \"block\", \
\"findings\": [{{\"severity\": \"block\" | \"high\" | \"medium\" | \"low\", \"title\": \"...\", \"detail\": \"...\", \"file\": \"path or null\"}}], \
\"rationale\": \"two or three sentences\"}}\n\
Use \"block\" only for: a test weakened or deleted to pass, a change that does something the human did not ask for and that is hard to reverse, \
or a claim of verification contradicted by the evidence. Use \"request_changes\" for real defects. Use \"approve\" when the change does what was asked and nothing above applies.",
        prompt = if scope.prompt.is_empty() { "(not recorded)" } else { &scope.prompt },
        summary = if scope.summary.is_empty() { "(nothing)" } else { &scope.summary },
        passed = scope.checks_passed,
        failed = scope.checks_failed,
        files = if files.is_empty() { "(none recorded)".to_string() } else { files },
        floor = floor_text,
        diff = if diff.is_empty() { "(no diff available: the turn has no snapshots)" } else { diff },
    )
}

/// Lenient JSON extraction: the first `{` to the last `}` of the answer.
pub fn parse_answer(text: &str) -> (Option<Verdict>, Vec<Finding>, String) {
    let start = text.find('{');
    let end = text.rfind('}');
    let (Some(a), Some(b)) = (start, end) else {
        return (None, Vec::new(), text.trim().to_string());
    };
    if b < a {
        return (None, Vec::new(), text.trim().to_string());
    }
    let Ok(v) = serde_json::from_str::<Value>(&text[a..=b]) else {
        return (None, Vec::new(), text.trim().to_string());
    };
    let verdict = v
        .get("verdict")
        .and_then(Value::as_str)
        .and_then(Verdict::parse);
    let findings = v
        .get("findings")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    let title = f.get("title")?.as_str()?.to_string();
                    let severity = f
                        .get("severity")
                        .and_then(Value::as_str)
                        .unwrap_or("medium")
                        .to_string();
                    Some(Finding {
                        severity,
                        rule: "model".into(),
                        title,
                        detail: f
                            .get("detail")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        file: f.get("file").and_then(Value::as_str).map(String::from),
                        deterministic: false,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let rationale = v
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    (verdict, findings, rationale)
}

/// Review one turn with the engine adapter injected (tests use a fake).
#[allow(clippy::too_many_arguments)]
pub fn review_with_runner(
    testigo: &TestigoService,
    project_root: &str,
    term_id: &str,
    turn_id: Option<&str>,
    reviewer: (Engine, Option<String>),
    runner: &dyn EngineRunner,
    ts: i64,
) -> AppResult<ReviewVerdict> {
    let scope = scope_for(testigo, project_root, term_id, turn_id)?;
    let floor = deterministic_findings(&scope);
    let diff = diff_text(Path::new(project_root), &scope);
    let prompt = build_prompt(&scope, &diff, &floor);
    let (engine, model) = reviewer;
    let run = agent_run::run_with_runner(
        &RunSpec {
            engine,
            cwd: Path::new(project_root),
            prompt: &prompt,
            policy: if engine == Engine::Claude {
                ToolPolicy::Plan
            } else {
                ToolPolicy::ReadOnly
            },
            model: model.as_deref(),
            resume: None,
            idle_timeout: agent_run::default_idle_timeout(),
            on_activity: None,
        },
        runner,
    )?;
    let (model_verdict, mut findings, rationale) = parse_answer(&run.text);
    let mut all = floor.clone();
    all.append(&mut findings);
    // The model can only tighten the floor, never loosen it.
    let verdict = floor_from(&floor).stricter(model_verdict.unwrap_or(Verdict::Approve));
    let reviewer_name = format!(
        "reviewer:{}",
        match engine {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
        }
    );
    let mut out = ReviewVerdict {
        verdict,
        findings: all,
        reviewer: reviewer_name.clone(),
        reviewer_engine: engine,
        reviewer_model: model.clone(),
        producer_model: scope.producer_model.clone(),
        term_id: scope.term_id.clone(),
        turn_id: scope.turn_id.clone(),
        case_id: scope.case_id.clone(),
        files: scope.files.len(),
        tokens: run.tokens,
        cost_usd: run.cost_usd,
        rationale: rationale.chars().take(2000).collect(),
        seq: None,
    };
    let payload = json!({
        "verdict": out.verdict,
        "findings": out.findings,
        "reviewer": { "engine": engine, "model": model },
        "producer": { "model": scope.producer_model },
        "scope": { "preSha": scope.pre_sha, "postSha": scope.post_sha, "files": scope.files.len(), "checksPassed": scope.checks_passed },
        "rationale": out.rationale,
        "tokens": run.tokens,
        "costUsd": run.cost_usd,
        "deterministic": !floor.is_empty(),
    });
    // D14: the reviewer's own identity, once per project.
    let engine_name = match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    };
    let card = crate::services::identity_service::card_for_reviewer(
        engine_name,
        model.as_deref(),
        ts,
        &crate::commands::identity::issuer(),
    );
    let _ = testigo.on_agent_identity(
        project_root,
        ts,
        &card.agent_id,
        Some(term_id),
        crate::services::identity_service::ledger_payload(&card, None),
    );
    let ev = testigo.on_review(
        project_root,
        ts,
        &scope.case_id,
        Some(&scope.turn_id),
        term_id,
        "review_verdict",
        &format!("agent:{reviewer_name}"),
        payload,
    )?;
    out.seq = Some(ev.seq);
    Ok(out)
}

/// Review with the real adapters: the reviewer is picked to differ from the
/// producer (engine first, model family second).
pub fn review(
    testigo: &TestigoService,
    project_root: &str,
    term_id: &str,
    turn_id: Option<&str>,
    ts: i64,
) -> AppResult<ReviewVerdict> {
    let scope = scope_for(testigo, project_root, term_id, turn_id)?;
    let reviewer = pick_reviewer(
        scope.producer_engine,
        scope.producer_model.as_deref(),
        engine_available(&claude_cli::codex_bin()),
        engine_available(&claude_cli::bin()),
    );
    let runner = crate::services::engine_runner::runner_for(reviewer.0);
    review_with_runner(
        testigo,
        project_root,
        term_id,
        turn_id,
        reviewer,
        runner,
        ts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::engine_runner::{RunCtx, TurnOutput};

    fn scope(files: &[(&str, &str)], prompt: &str, summary: &str, passed: usize) -> ReviewScope {
        ReviewScope {
            term_id: "t1".into(),
            turn_id: "turn-1".into(),
            case_id: "case".into(),
            prompt: prompt.into(),
            summary: summary.into(),
            files: files
                .iter()
                .map(|(s, p)| (s.to_string(), p.to_string()))
                .collect(),
            checks_passed: passed,
            ..Default::default()
        }
    }

    #[test]
    fn a_test_edited_with_its_source_blocks_unless_tests_were_asked_for() {
        let s = scope(
            &[("M", "src/api.ts"), ("M", "tests/api.test.ts")],
            "fix the 500 on login",
            "",
            1,
        );
        let f = deterministic_findings(&s);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "test_edited");
        assert_eq!(f[0].severity, "block");
        assert_eq!(floor_from(&f), Verdict::Block);

        let s = scope(
            &[("M", "src/api.ts"), ("M", "tests/api.test.ts")],
            "add a test for login",
            "",
            1,
        );
        assert_eq!(deterministic_findings(&s)[0].severity, "high");
        // Only tests, or only sources: nothing to flag.
        assert!(deterministic_findings(&scope(&[("A", "tests/x.test.ts")], "", "", 0)).is_empty());
        assert!(deterministic_findings(&scope(&[("M", "src/x.ts")], "", "", 0)).is_empty());
    }

    #[test]
    fn a_green_claim_without_a_witnessed_check_is_flagged() {
        let s = scope(&[("M", "src/x.ts")], "fix", "Done, all tests pass.", 0);
        let f = deterministic_findings(&s);
        assert_eq!(f[0].rule, "unverified_claim");
        let s = scope(&[("M", "src/x.ts")], "fix", "Done, all tests pass.", 1);
        assert!(deterministic_findings(&s).is_empty());
    }

    #[test]
    fn the_reviewer_differs_from_the_producer() {
        assert_eq!(
            pick_reviewer(Engine::Claude, Some("claude-opus-5-5"), true, true),
            (Engine::Codex, None)
        );
        assert_eq!(
            pick_reviewer(Engine::Codex, None, true, true),
            (Engine::Claude, None)
        );
        assert_eq!(
            pick_reviewer(Engine::Claude, Some("claude-opus-5-5"), false, true),
            (Engine::Claude, Some("sonnet".into()))
        );
        assert_eq!(
            pick_reviewer(Engine::Claude, Some("claude-sonnet-5-5"), false, true),
            (Engine::Claude, Some("opus".into()))
        );
    }

    #[test]
    fn parse_answer_is_lenient_and_the_model_cannot_loosen_the_floor() {
        let (v, f, r) = parse_answer("Sure! {\"verdict\":\"approve\",\"findings\":[{\"severity\":\"low\",\"title\":\"nit\"}],\"rationale\":\"fine\"} thanks");
        assert_eq!(v, Some(Verdict::Approve));
        assert_eq!(f.len(), 1);
        assert_eq!(r, "fine");
        assert_eq!(Verdict::Block.stricter(Verdict::Approve), Verdict::Block);
        assert_eq!(
            Verdict::Approve.stricter(Verdict::RequestChanges),
            Verdict::RequestChanges
        );
        let (v, f, _) = parse_answer("no json here");
        assert_eq!(v, None);
        assert!(f.is_empty());
    }

    struct FakeReviewer(&'static str);
    impl EngineRunner for FakeReviewer {
        fn run(
            &self,
            ctx: &RunCtx,
            _on: &crate::services::engine_runner::ActivitySink,
        ) -> AppResult<TurnOutput> {
            assert!(ctx.prompt.contains("adversarial code reviewer"));
            assert!(ctx.prompt.contains("fix the 500"));
            Ok(TurnOutput {
                text: self.0.to_string(),
                session_id: None,
                tokens: 321,
                cost_usd: 0.02,
            })
        }
    }

    /// End to end against the ledger: a turn that edited a test lands a
    /// `review_verdict` block under the reviewer's own identity, even though
    /// the model said approve.
    #[test]
    fn a_review_lands_in_the_ledger_under_the_reviewers_identity() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("ac-review-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
        let svc = TestigoService::new();
        let root = base.to_string_lossy().to_string();
        svc.on_session_model(&root, 1, Some("t1"), "s1", "claude-opus-5-5")
            .ok();
        let prompt = svc
            .on_prompt(
                &root,
                2,
                Some("t1"),
                Some("s1"),
                Some("fix the 500 on login"),
                None,
                None,
            )
            .unwrap();
        svc.on_turn_end(
            &root,
            3,
            Some("t1"),
            Some("s1"),
            json!({
                "summary": "Fixed; tests pass.",
                "filesChanged": [{ "status": "M", "path": "src/api.ts" }, { "status": "M", "path": "tests/api.test.ts" }]
            }),
        )
        .unwrap();
        let v = review_with_runner(
            &svc,
            &root,
            "t1",
            None,
            (Engine::Codex, None),
            &FakeReviewer("{\"verdict\":\"approve\",\"findings\":[],\"rationale\":\"looks fine\"}"),
            4,
        )
        .unwrap();
        assert_eq!(v.verdict, Verdict::Block, "the floor wins over the model");
        assert_eq!(v.turn_id, prompt.turn_id.unwrap());
        assert!(v.findings.iter().any(|f| f.rule == "test_edited"));
        assert!(v.findings.iter().any(|f| f.rule == "unverified_claim"));
        assert_eq!(v.reviewer, "reviewer:codex");
        let events = svc.list(&root, None, None).unwrap();
        let ev = events.iter().find(|e| e.kind == "review_verdict").unwrap();
        assert_eq!(ev.actor, "agent:reviewer:codex");
        assert_eq!(ev.turn_id, v.turn_id.clone().into());
        assert_eq!(ev.payload["verdict"], "block");
        assert_eq!(ev.payload["producer"]["model"], "claude-opus-5-5");
        assert!(svc.verify(&root).unwrap().ok);
        std::fs::remove_dir_all(&base).ok();
    }
}
