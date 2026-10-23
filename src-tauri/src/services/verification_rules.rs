//! The verification contract, computed — never asserted — from a turn's own
//! ledger lines (COMPETENCIA-2026-10 F1c).
//!
//! The question a reviewer asks first is "did the tests actually run, and
//! did they pass AFTER the last edit?" — and the answer the agent gives in
//! its closing words is the least reliable line of the turn ("ctest ran zero
//! tests and exited 0, and the agent then told the user that all tests
//! pass"). So the verdict here is deterministic over witnessed events:
//! `tool_result`, `check_run` (F1a: `passed` only with a known exit 0) and
//! `turn_end`. A model may only ever HARDEN it (the reviewer); nothing
//! softens it.
//!
//! The same rules exist as `.github/testigo/verification-rules.mjs` (the PR
//! gate) and inside `testigo-verifier.html` (the standalone verifier), so a
//! third party recomputes the status from a proof packet without trusting
//! the console. `fixtures/verification/cases.json` keeps the three in step.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::services::testigo_service::{canonical_tool, ProofEvent};

/// Bumped whenever a rule changes meaning; recorded in every event so an old
/// packet is judged by the rules it was written under.
pub const RULES_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The turn was closed without a per-tool witness (no hook/plugin close
    /// and no tool line): a claim here is listed, not judged.
    Unwitnessed,
    /// The agent claimed green and a witnessed check failed or ran nothing.
    ClaimContradicted,
    /// The agent claimed green and no check passed with a known exit 0.
    UnverifiedClaim,
    /// A check passed, but the agent edited files AFTER it ran.
    StaleCheck,
    /// A check passed with exit 0 after the last edit: the claim holds.
    Verified,
    /// Checks ran, no claim was made.
    Checked,
    /// Neither a claim nor a check.
    NoClaim,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Unwitnessed => "unwitnessed",
            Status::ClaimContradicted => "claim_contradicted",
            Status::UnverifiedClaim => "unverified_claim",
            Status::StaleCheck => "stale_check",
            Status::Verified => "verified",
            Status::Checked => "checked",
            Status::NoClaim => "no_claim",
        }
    }
}

/// A phrase in the closing words that asserts a green outcome. `negated`
/// means a guard word precedes it in the same sentence ("no pude correr",
/// "didn't run", "should pass"): listed for the record, not counted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub phrase: String,
    pub lang: String,
    pub negated: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checks {
    pub passed: u32,
    pub failed: u32,
    pub empty: u32,
    pub unknown: u32,
    pub interrupted: u32,
}

impl Checks {
    pub fn total(&self) -> u32 {
        self.passed + self.failed + self.empty + self.unknown + self.interrupted
    }
}

/// What the turn's lines say, folded once.
#[derive(Debug, Default, Clone)]
pub struct TurnFacts {
    pub tool_results: u32,
    pub commands: u32,
    pub failed_commands: u32,
    pub checks: Checks,
    /// check kind → worst status seen for it.
    pub gates_ran: BTreeMap<String, String>,
    pub last_edit_seq: Option<u64>,
    pub last_check_seq: Option<u64>,
    /// The last check that PASSED (exit 0 known, ran something).
    pub last_good_check_seq: Option<u64>,
    pub summary: String,
    pub superseded: bool,
}

impl TurnFacts {
    /// `per_tool` when hooks/plugin saw the turn's tools; `turn_boundary`
    /// when the turn was ended by the next prompt and no tool line exists —
    /// the console cannot know what ran.
    pub fn coverage(&self) -> &'static str {
        if self.superseded && self.tool_results == 0 {
            "turn_boundary"
        } else {
            "per_tool"
        }
    }
}

fn worse(a: &str, b: &str) -> bool {
    // failed > empty > unknown > interrupted > passed
    let rank = |s: &str| match s {
        "failed" => 4,
        "empty" => 3,
        "unknown" => 2,
        "interrupted" => 1,
        _ => 0,
    };
    rank(a) > rank(b)
}

/// Fold one turn's events (seq order) into facts.
pub fn fold<'a>(events: impl IntoIterator<Item = &'a ProofEvent>) -> TurnFacts {
    let mut f = TurnFacts::default();
    for e in events {
        let p = &e.payload;
        match e.kind.as_str() {
            "tool_result" => {
                f.tool_results += 1;
                let canonical = p
                    .get("toolCanonical")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        p.get("tool")
                            .and_then(Value::as_str)
                            .map(|t| canonical_tool(t).to_string())
                    })
                    .unwrap_or_else(|| "other".into());
                match canonical.as_str() {
                    "shell" => {
                        f.commands += 1;
                        let failed = p.get("failed").and_then(Value::as_bool) == Some(true)
                            || p.get("exitCode")
                                .and_then(Value::as_i64)
                                .is_some_and(|c| c != 0);
                        if failed {
                            f.failed_commands += 1;
                        }
                    }
                    "edit" => f.last_edit_seq = Some(e.seq),
                    _ => {}
                }
            }
            "check_run" => {
                let status = p.get("status").and_then(Value::as_str).unwrap_or("unknown");
                match status {
                    "passed" => f.checks.passed += 1,
                    "failed" => f.checks.failed += 1,
                    "empty" => f.checks.empty += 1,
                    "interrupted" => f.checks.interrupted += 1,
                    _ => f.checks.unknown += 1,
                }
                f.last_check_seq = Some(e.seq);
                if status == "passed" {
                    f.last_good_check_seq = Some(e.seq);
                }
                let kind = p
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("test")
                    .to_string();
                let entry = f
                    .gates_ran
                    .entry(kind)
                    .or_insert_with(|| status.to_string());
                if worse(status, entry) {
                    *entry = status.to_string();
                }
            }
            "turn_end" => {
                f.summary = p
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                f.superseded = p.get("superseded").and_then(Value::as_bool) == Some(true);
            }
            _ => {}
        }
    }
    f
}

/// (phrase, lang). Lower-case, matched as substrings of the lower-cased
/// closing words. Deliberately literal: the list grows with real cases, and
/// every addition lands in cases.json.
const CLAIM_PHRASES: &[(&str, &str)] = &[
    ("all tests pass", "en"),
    ("tests pass", "en"),
    ("test passes", "en"),
    ("tests are passing", "en"),
    ("tests passing", "en"),
    ("test suite passes", "en"),
    ("tests succeed", "en"),
    ("all checks pass", "en"),
    ("checks pass", "en"),
    ("all green", "en"),
    ("ci is green", "en"),
    ("build passes", "en"),
    ("build succeeds", "en"),
    ("lint passes", "en"),
    ("lint is clean", "en"),
    ("typecheck passes", "en"),
    ("type checks pass", "en"),
    ("todos los tests pasan", "es"),
    ("los tests pasan", "es"),
    ("pasan los tests", "es"),
    ("las pruebas pasan", "es"),
    ("pasan las pruebas", "es"),
    ("pasan todos los tests", "es"),
    ("todo en verde", "es"),
    ("todo verde", "es"),
    ("el build pasa", "es"),
    ("compila sin errores", "es"),
    ("lint limpio", "es"),
    ("el lint pasa", "es"),
    ("typecheck pasa", "es"),
    ("sin errores de tipos", "es"),
    ("la suite pasa", "es"),
    ("todos os testes passam", "pt"),
    ("os testes passam", "pt"),
    ("testes passando", "pt"),
    ("tudo verde", "pt"),
    ("build passa", "pt"),
    ("lint limpo", "pt"),
];

/// Words that, earlier in the same sentence, turn a claim into a non-claim:
/// negation ("no pude correr los tests"), inability, or a conditional
/// ("should pass", "deberían pasar").
const GUARDS: &[&str] = &[
    "not ",
    "n't",
    "never",
    "without",
    "unable",
    "couldn",
    "didn",
    "did not",
    "do not",
    "no longer",
    "should",
    "would",
    "will",
    "once",
    "if ",
    "when ",
    "after you",
    "expect",
    "hopefully",
    "probably",
    "no ",
    "sin ",
    "nunca",
    "no pude",
    "no corr",
    "deber",
    "podr",
    "cuando",
    "una vez",
    "espero",
    "não",
    "nao ",
    "sem ",
    "nunca",
    "deveria",
    "deveriam",
    "poderia",
    "quando",
];

fn sentences(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(['.', '\n', ';', '!', '?'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Every green claim in the closing words, negated or not.
pub fn claims(summary: &str) -> Vec<Claim> {
    let mut out: Vec<Claim> = Vec::new();
    for sentence in sentences(summary) {
        for (phrase, lang) in CLAIM_PHRASES {
            let Some(at) = sentence.find(phrase) else {
                continue;
            };
            // A longer phrase already matched here ("all tests pass" ⊃ "tests pass").
            if out.iter().any(|c| {
                c.phrase != *phrase && c.phrase.contains(phrase) && sentence.contains(&c.phrase)
            }) {
                continue;
            }
            let before = &sentence[..at];
            let negated = GUARDS.iter().any(|g| before.contains(g));
            out.push(Claim {
                phrase: (*phrase).to_string(),
                lang: (*lang).to_string(),
                negated,
            });
        }
    }
    out
}

/// The verdict and a one-line reason, from facts and claims. Rule order is
/// the contract: the first that applies wins.
pub fn verdict(facts: &TurnFacts, claims: &[Claim]) -> (Status, String) {
    let live: Vec<&Claim> = claims.iter().filter(|c| !c.negated).collect();
    let c = &facts.checks;
    if live.is_empty() {
        return if c.total() > 0 {
            (
                Status::Checked,
                format!(
                    "no claim; {} check{} ran ({} passed)",
                    c.total(),
                    if c.total() == 1 { "" } else { "s" },
                    c.passed
                ),
            )
        } else {
            (Status::NoClaim, "no claim, no check".into())
        };
    }
    let what = live[0].phrase.clone();
    if facts.coverage() == "turn_boundary" {
        return (
            Status::Unwitnessed,
            format!("\"{what}\" — the turn was not witnessed per tool call, so the claim is listed, not judged"),
        );
    }
    if c.failed > 0 || c.empty > 0 {
        let mut why = Vec::new();
        if c.failed > 0 {
            why.push(format!(
                "{} check{} failed",
                c.failed,
                if c.failed == 1 { "" } else { "s" }
            ));
        }
        if c.empty > 0 {
            why.push(format!(
                "{} runner{} exited 0 having run zero tests",
                c.empty,
                if c.empty == 1 { "" } else { "s" }
            ));
        }
        return (
            Status::ClaimContradicted,
            format!("\"{what}\" but {}", why.join(" and ")),
        );
    }
    let Some(good) = facts.last_good_check_seq else {
        let why = if c.unknown > 0 {
            "the only checks seen had no visible exit code"
        } else if c.interrupted > 0 {
            "the only checks seen were interrupted"
        } else {
            "no check ran in this turn"
        };
        return (Status::UnverifiedClaim, format!("\"{what}\" but {why}"));
    };
    if facts.last_edit_seq.is_some_and(|edit| edit > good) {
        return (
            Status::StaleCheck,
            format!("\"{what}\" but files were edited after the last passing check"),
        );
    }
    (
        Status::Verified,
        format!(
            "\"{what}\" and {} check{} passed with exit 0 after the last edit",
            c.passed,
            if c.passed == 1 { "" } else { "s" }
        ),
    )
}

/// The `verification` ledger payload. `project_gates` are the kinds the
/// project is known to have (gate_discovery) so "tests: not_run" can be said.
pub fn payload(
    facts: &TurnFacts,
    claims: &[Claim],
    status: Status,
    reason: &str,
    project_gates: &[(String, String)],
) -> Value {
    let mut gates: BTreeMap<String, Value> = BTreeMap::new();
    for (kind, command) in project_gates {
        let ran = facts.gates_ran.get(kind).cloned();
        gates.insert(
            kind.clone(),
            json!({ "command": command, "status": ran.unwrap_or_else(|| "not_run".into()) }),
        );
    }
    for (kind, status) in &facts.gates_ran {
        gates
            .entry(kind.clone())
            .or_insert_with(|| json!({ "status": status }));
    }
    json!({
        "rulesVersion": RULES_VERSION,
        "status": status.as_str(),
        "reason": reason,
        "coverage": facts.coverage(),
        "commands": { "total": facts.commands, "failed": facts.failed_commands },
        "checks": facts.checks,
        "lastEditSeq": facts.last_edit_seq,
        "lastCheckSeq": facts.last_check_seq,
        "claims": claims,
        "gates": gates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../../fixtures/verification/cases.json");

    fn event(seq: u64, kind: &str, payload: Value) -> ProofEvent {
        ProofEvent {
            seq,
            ts: seq as i64,
            case_id: "c".into(),
            turn_id: Some("t".into()),
            kind: kind.into(),
            term_id: Some("term".into()),
            session_id: None,
            actor: "agent".into(),
            payload,
            prev_hash: String::new(),
            hash: String::new(),
        }
    }

    #[test]
    fn every_shared_vector_agrees_with_the_rules() {
        let doc: Value = serde_json::from_str(CASES).unwrap();
        for case in doc["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let events: Vec<ProofEvent> = case["events"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    event(
                        i as u64 + 1,
                        e["kind"].as_str().unwrap(),
                        e["payload"].clone(),
                    )
                })
                .collect();
            let facts = fold(&events);
            let claims = claims(&facts.summary);
            let (status, reason) = verdict(&facts, &claims);
            assert_eq!(
                status.as_str(),
                case["expected"]["status"],
                "{name}: {reason}"
            );
            assert_eq!(facts.coverage(), case["expected"]["coverage"], "{name}");
            assert_eq!(
                claims.iter().filter(|c| !c.negated).count() as u64,
                case["expected"]["claims"].as_u64().unwrap(),
                "{name}: claims {claims:?}"
            );
            assert!(!reason.is_empty(), "{name}");
        }
    }

    #[test]
    fn claims_are_found_per_sentence_with_their_guards() {
        let c = claims("Refactored. All tests pass now, and lint is clean.");
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|c| !c.negated));
        let c = claims("I did not run them, but tests pass locally per the user");
        assert_eq!(c.len(), 1);
        assert!(c[0].negated);
        let c = claims("Los tests pasan. No pude correr el lint, el lint pasa en CI seguramente");
        assert_eq!(c.iter().filter(|c| !c.negated).count(), 1);
        assert_eq!(c.iter().filter(|c| c.negated).count(), 1);
        assert!(claims("Nothing to report.").is_empty());
        // "all tests pass" is not also counted as "tests pass".
        assert_eq!(claims("all tests pass").len(), 1);
    }

    #[test]
    fn the_payload_names_project_gates_that_did_not_run() {
        let facts = fold(&[
            event(1, "check_run", json!({"kind": "lint", "status": "passed"})),
            event(2, "turn_end", json!({"summary": "done"})),
        ]);
        let p = payload(
            &facts,
            &[],
            Status::Checked,
            "x",
            &[
                ("test".into(), "npm test".into()),
                ("lint".into(), "npm run lint".into()),
            ],
        );
        assert_eq!(p["gates"]["test"]["status"], "not_run");
        assert_eq!(p["gates"]["test"]["command"], "npm test");
        assert_eq!(p["gates"]["lint"]["status"], "passed");
        assert_eq!(p["rulesVersion"], RULES_VERSION);
        assert_eq!(p["coverage"], "per_tool");
    }
}
