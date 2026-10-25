//! F7 (COMPETENCIA-2026-10): the pull request as the ledger sees it.
//!
//! Proof-on-PR (P2) binds a packet to the pushed HEAD; what was missing is
//! the other direction — from the PR back to the sessions, turns and
//! verdicts that produced its commits, with the hosting side's CI state
//! next to them. This module reads the PR through the `gh` CLI (no token
//! of our own, no bot: the user's own auth, like Feedback does), walks the
//! branch's commits against the ledger's `commit` and `verification`
//! lines, and answers the one question merge-when-green needs: is every
//! turn behind this branch verified, or does a human have to say so?
//!
//! Nothing here is estimated. A commit the ledger never saw is `unbound`,
//! a turn without a verdict is `unverified`; both block the automatic
//! gate and hand the decision to the human, who records it.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::git_service;
use crate::services::proc;
use crate::services::testigo_service::ProofEvent;

/// Longest comment body carried into the console and the auto-fix prompt.
pub const COMMENT_BODY_CAP: usize = 600;
/// Inline review comments fetched per PR (newest first after sorting).
pub const COMMENTS_CAP: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    /// pending · success · failure · skipped
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrComment {
    pub id: u64,
    pub author: String,
    pub bot: bool,
    /// inline (a line of the diff) · review (a review's body) · issue
    pub kind: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
    pub title: String,
    /// OPEN · MERGED · CLOSED
    pub state: String,
    pub is_draft: bool,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    /// GitHub's mergeStateStatus: CLEAN · BLOCKED · BEHIND · DIRTY · UNSTABLE · UNKNOWN…
    pub merge_state: String,
    pub review_decision: String,
    pub checks: Vec<PrCheck>,
    /// none · pending · green · red — one word for the row and the card.
    pub ci: String,
    pub comments: Vec<PrComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommitEvidence {
    pub sha: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term_id: Option<String>,
    /// Who committed as the ledger saw it: human (GUI) · agent (terminal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// The turn's verdict (F1c status), `accepted` when a human accepted
    /// it (F1d), `unbound` when the ledger never saw the commit,
    /// `unverified` when the turn closed without a verdict.
    pub verification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BranchEvidence {
    pub branch: String,
    pub base: String,
    pub commits: Vec<CommitEvidence>,
    pub verified: usize,
    pub accepted: usize,
    pub unverified: usize,
    pub contradicted: usize,
    pub unbound: usize,
    /// Every commit bound to a turn whose verdict is green (or accepted by
    /// a human): the only state in which merge-when-green needs no word.
    pub all_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrOverview {
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<PrInfo>,
    pub evidence: BranchEvidence,
    /// ok · no_pr · missing (gh not installed) · error
    pub gh: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `gh pr view --json …` for the checked-out branch, as a value. `Ok(None)`
/// when there is no PR for the branch; `Err` when gh is missing or failed.
fn gh_pr_view(repo: &Path) -> AppResult<Option<Value>> {
    let out = proc::command("gh")
        .args([
            "pr",
            "view",
            "--json",
            "number,url,title,state,isDraft,headRefName,baseRefName,headRefOid,mergeStateStatus,reviewDecision,statusCheckRollup,reviews,comments",
        ])
        .current_dir(repo)
        .output()
        .map_err(|e| {
            AppError::Other(format!(
                "gh CLI not available ({e}). Install: https://cli.github.com/"
            ))
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if stderr.contains("no pull requests found") || stderr.contains("not found") {
            return Ok(None);
        }
        return Err(AppError::Other(format!("gh pr view failed: {stderr}")));
    }
    let v: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| AppError::Other(format!("gh pr view: unreadable JSON ({e})")))?;
    Ok(Some(v))
}

/// Inline review comments (`pulls/{n}/comments`), best-effort: a PR
/// without them still shows. `slug` is `owner/repo`.
fn gh_inline_comments(repo: &Path, slug: &str, number: u64) -> Vec<PrComment> {
    let out = proc::command("gh")
        .args([
            "api",
            &format!("repos/{slug}/pulls/{number}/comments?per_page=100"),
        ])
        .current_dir(repo)
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    serde_json::from_slice::<Value>(&out.stdout)
        .map(|v| parse_inline_comments(&v))
        .unwrap_or_default()
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn cap(body: &str) -> String {
    crate::services::text::cap_chars(body, COMMENT_BODY_CAP)
}

fn author_of(v: &Value) -> (String, bool) {
    let login = v
        .get("author")
        .or_else(|| v.get("user"))
        .and_then(|a| a.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let typed_bot = v
        .get("user")
        .and_then(|u| u.get("type"))
        .and_then(Value::as_str)
        == Some("Bot");
    // GraphQL authors carry no `[bot]` suffix (Codex reviews as
    // `chatgpt-codex-connector`); the flag only steers emphasis in the UI.
    let lower = login.to_ascii_lowercase();
    let bot = typed_bot || lower.contains("bot") || lower.ends_with("-connector");
    (login, bot)
}

/// One check as `statusCheckRollup` lists it: a `CheckRun` (status +
/// conclusion) or a legacy `StatusContext` (state). Anything not finished
/// is pending; a finished run without a conclusion we treat as pending
/// too rather than inventing a verdict.
pub fn parse_check(v: &Value) -> PrCheck {
    let name = if v.get("name").and_then(Value::as_str).is_some() {
        s(v, "name")
    } else {
        s(v, "context")
    };
    let url = v
        .get("detailsUrl")
        .or_else(|| v.get("targetUrl"))
        .and_then(Value::as_str)
        .map(String::from);
    let status = if v.get("__typename").and_then(Value::as_str) == Some("StatusContext") {
        match s(v, "state").as_str() {
            "SUCCESS" => "success",
            "FAILURE" | "ERROR" => "failure",
            _ => "pending",
        }
    } else if s(v, "status") != "COMPLETED" {
        "pending"
    } else {
        match s(v, "conclusion").as_str() {
            "SUCCESS" => "success",
            "NEUTRAL" | "SKIPPED" => "skipped",
            "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => {
                "failure"
            }
            _ => "pending",
        }
    };
    PrCheck {
        name,
        status: status.into(),
        url,
    }
}

/// One word from the checks: a failure anywhere is red; otherwise anything
/// still running is pending; otherwise green; no checks is none.
pub fn ci_of(checks: &[PrCheck]) -> &'static str {
    if checks.is_empty() {
        return "none";
    }
    if checks.iter().any(|c| c.status == "failure") {
        return "red";
    }
    if checks.iter().any(|c| c.status == "pending") {
        return "pending";
    }
    "green"
}

/// `gh pr view --json` → the PR. Reviews with a body and issue comments
/// become comments too (kind `review` / `issue`); inline ones are fetched
/// separately and merged by the caller.
pub fn parse_pr_view(v: &Value) -> PrInfo {
    let checks: Vec<PrCheck> = v
        .get("statusCheckRollup")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(parse_check).collect())
        .unwrap_or_default();
    let mut comments = Vec::new();
    for (key, kind) in [("reviews", "review"), ("comments", "issue")] {
        for c in v.get(key).and_then(Value::as_array).into_iter().flatten() {
            let body = cap(&s(c, "body"));
            if body.is_empty() {
                continue;
            }
            let (author, bot) = author_of(c);
            comments.push(PrComment {
                id: id_of(c),
                author,
                bot,
                kind: kind.into(),
                body,
                path: None,
                line: None,
                url: c.get("url").and_then(Value::as_str).map(String::from),
                created_at: if kind == "review" {
                    s(c, "submittedAt")
                } else {
                    s(c, "createdAt")
                },
            });
        }
    }
    PrInfo {
        number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
        url: s(v, "url"),
        title: s(v, "title"),
        state: s(v, "state"),
        is_draft: v.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        head_ref: s(v, "headRefName"),
        base_ref: s(v, "baseRefName"),
        head_sha: s(v, "headRefOid"),
        merge_state: s(v, "mergeStateStatus"),
        review_decision: s(v, "reviewDecision"),
        ci: ci_of(&checks).into(),
        checks,
        comments,
    }
}

/// GraphQL node ids (`IC_kwDO…`) are strings; REST ids are numbers. Both
/// hash to a stable u64 so the auto-fix record can name what it enqueued.
fn id_of(v: &Value) -> u64 {
    match v.get("id") {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(st)) => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            st.hash(&mut h);
            h.finish() >> 1
        }
        _ => 0,
    }
}

/// REST `pulls/{n}/comments` → inline comments, oldest first, capped.
pub fn parse_inline_comments(v: &Value) -> Vec<PrComment> {
    let mut out: Vec<PrComment> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let body = cap(&s(c, "body"));
            if body.is_empty() {
                return None;
            }
            let (author, bot) = author_of(c);
            Some(PrComment {
                id: id_of(c),
                author,
                bot,
                kind: "inline".into(),
                body,
                path: c.get("path").and_then(Value::as_str).map(String::from),
                line: c
                    .get("line")
                    .and_then(Value::as_u64)
                    .or_else(|| c.get("original_line").and_then(Value::as_u64)),
                url: c.get("html_url").and_then(Value::as_str).map(String::from),
                created_at: s(c, "created_at"),
            })
        })
        .collect();
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    out.truncate(COMMENTS_CAP);
    out
}

/// Commits on the branch that are not on `base`, newest first.
pub fn branch_commits(repo: &Path, base: &str) -> AppResult<Vec<(String, String)>> {
    let range = format!("{base}..HEAD");
    let out = proc::command("git")
        .args(["log", "--format=%H%x1f%s", "--max-count=200", &range])
        .current_dir(repo)
        .output()?;
    if !out.status.success() {
        // No such base ref (shallow clone, no fetch yet): fall back to HEAD
        // alone so the overview still says what it can.
        let head = git_service::head_sha(repo)?;
        let subject = proc::command("git")
            .args(["log", "-1", "--format=%s", "HEAD"])
            .current_dir(repo)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        return Ok(vec![(head, subject)]);
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (sha, subject) = l.split_once('\u{1f}')?;
            (!sha.is_empty()).then(|| (sha.to_string(), subject.to_string()))
        })
        .collect())
}

const GREEN: [&str; 3] = ["verified", "checked", "no_claim"];

/// Walk the branch's commits against the ledger: each commit's case, turn
/// and the turn's last verdict (plus the human's acceptance of it). Pure,
/// so the vectors live in tests.
pub fn evidence_for(
    branch: &str,
    base: &str,
    commits: &[(String, String)],
    events: &[ProofEvent],
) -> BranchEvidence {
    let mut out = Vec::with_capacity(commits.len());
    let (mut verified, mut accepted, mut unverified, mut contradicted, mut unbound) =
        (0, 0, 0, 0, 0);
    for (sha, subject) in commits {
        let commit = events.iter().rev().find(|e| {
            e.kind == "commit"
                && e.payload.get("sha").and_then(Value::as_str) == Some(sha.as_str())
                && e.case_id != "unbound"
        });
        let Some(c) = commit else {
            unbound += 1;
            out.push(CommitEvidence {
                sha: sha.clone(),
                subject: subject.clone(),
                case_id: None,
                turn_id: None,
                term_id: None,
                actor: None,
                verification: "unbound".into(),
            });
            continue;
        };
        let turn_id = c.turn_id.clone();
        let term_id = c.term_id.clone().or_else(|| {
            c.case_id
                .strip_prefix("term:")
                .map(String::from)
                .or_else(|| turn_term(events, turn_id.as_deref()))
        });
        let verdict = turn_id.as_deref().and_then(|t| {
            events
                .iter()
                .rev()
                .find(|e| e.kind == "verification" && e.turn_id.as_deref() == Some(t))
        });
        let status = match verdict {
            None => "unverified".to_string(),
            Some(v) => {
                let st = v
                    .payload
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unverified");
                let human_accepted = events.iter().any(|e| {
                    e.kind == "verification_decision"
                        && e.payload.get("verificationSeq").and_then(Value::as_u64) == Some(v.seq)
                        && e.payload.get("decision").and_then(Value::as_str) == Some("accept")
                });
                if GREEN.contains(&st) {
                    st.to_string()
                } else if human_accepted {
                    "accepted".into()
                } else {
                    st.to_string()
                }
            }
        };
        match status.as_str() {
            "verified" | "checked" | "no_claim" => verified += 1,
            "accepted" => accepted += 1,
            "claim_contradicted" => contradicted += 1,
            _ => unverified += 1,
        }
        out.push(CommitEvidence {
            sha: sha.clone(),
            subject: subject.clone(),
            case_id: Some(c.case_id.clone()),
            turn_id,
            term_id,
            actor: Some(c.actor.clone()),
            verification: status,
        });
    }
    let all_verified = !out.is_empty() && unbound == 0 && unverified == 0 && contradicted == 0;
    BranchEvidence {
        branch: branch.into(),
        base: base.into(),
        commits: out,
        verified,
        accepted,
        unverified,
        contradicted,
        unbound,
        all_verified,
    }
}

fn turn_term(events: &[ProofEvent], turn_id: Option<&str>) -> Option<String> {
    let t = turn_id?;
    events
        .iter()
        .find(|e| e.turn_id.as_deref() == Some(t) && e.term_id.is_some())
        .and_then(|e| e.term_id.clone())
}

/// The whole picture for the checked-out branch: PR (if gh finds one), its
/// CI and comments, and the ledger's evidence for every commit behind it.
pub fn overview(repo: &Path, events: &[ProofEvent]) -> AppResult<PrOverview> {
    let branch = git_service::current_branch_name(repo)?;
    let (pr, gh, error) = match gh_pr_view(repo) {
        Ok(Some(v)) => {
            let mut pr = parse_pr_view(&v);
            if let Some(slug) = git_service::remote_slug(repo, "origin") {
                let mut inline = gh_inline_comments(repo, &slug, pr.number);
                inline.append(&mut pr.comments);
                pr.comments = inline;
            }
            (Some(pr), "ok".to_string(), None)
        }
        Ok(None) => (None, "no_pr".into(), None),
        Err(e) => {
            let msg = e.to_string();
            let gh = if msg.contains("not available") {
                "missing"
            } else {
                "error"
            };
            (None, gh.into(), Some(msg))
        }
    };
    let base_name = pr
        .as_ref()
        .map(|p| p.base_ref.clone())
        .or_else(|| git_service::default_branch_name(repo, "origin"))
        .unwrap_or_else(|| "main".into());
    // Prefer the remote's view of the base so a stale local main doesn't
    // claim commits already merged.
    let base_ref = if git_service::ref_exists(repo, &format!("refs/remotes/origin/{base_name}")) {
        format!("origin/{base_name}")
    } else {
        base_name.clone()
    };
    let commits = branch_commits(repo, &base_ref)?;
    let evidence = evidence_for(&branch, &base_name, &commits, events);
    Ok(PrOverview {
        branch,
        pr,
        evidence,
        gh,
        error,
    })
}

/// `gh pr merge`: now when CI is green, else GitHub's own auto-merge
/// (needs branch protection with required checks; gh says so when it
/// can't). The gate — verified turns or a human's word — is the caller's.
pub fn merge(repo: &Path, number: u64, ci_green: bool) -> AppResult<String> {
    let n = number.to_string();
    let mut args = vec!["pr", "merge", &n, "--merge"];
    if !ci_green {
        args.push("--auto");
    }
    let out = proc::command("gh")
        .args(&args)
        .current_dir(repo)
        .output()
        .map_err(|e| {
            AppError::Other(format!(
                "gh CLI not available ({e}). Install: https://cli.github.com/"
            ))
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(AppError::Other(format!("gh pr merge failed: {stderr}")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// What `pr_link` carries: enough to find the PR and the sessions again
/// without re-asking GitHub.
pub fn link_payload(ov: &PrOverview) -> Value {
    let pr = ov.pr.as_ref();
    json!({
        "number": pr.map(|p| p.number),
        "url": pr.map(|p| p.url.clone()),
        "headRef": ov.branch,
        "baseRef": ov.evidence.base,
        "headSha": pr.map(|p| p.head_sha.clone()),
        "ci": pr.map(|p| p.ci.clone()),
        "commits": ov.evidence.commits.iter().map(|c| json!({
            "sha": c.sha, "turnId": c.turn_id, "termId": c.term_id, "verification": c.verification,
        })).collect::<Vec<_>>(),
        "allVerified": ov.evidence.all_verified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/pr")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn ev(
        seq: u64,
        kind: &str,
        case: &str,
        turn: Option<&str>,
        term: Option<&str>,
        payload: Value,
    ) -> ProofEvent {
        ProofEvent {
            seq,
            ts: seq as i64 * 1000,
            case_id: case.into(),
            turn_id: turn.map(String::from),
            kind: kind.into(),
            term_id: term.map(String::from),
            session_id: None,
            actor: if kind == "commit" {
                "agent".into()
            } else {
                "console".into()
            },
            payload,
            prev_hash: String::new(),
            hash: String::new(),
        }
    }

    /// gh 2.46 `pr view --json`: every check state maps to one of four
    /// words, a failure anywhere makes the PR red, reviews and issue
    /// comments with a body become comments with their kind.
    #[test]
    fn parses_gh_pr_view_checks_and_comments() {
        let pr = parse_pr_view(&fixture("gh-2.46-pr-view.json"));
        assert_eq!(pr.number, 269);
        assert_eq!(pr.head_ref, "feat/policy-f6");
        assert_eq!(pr.base_ref, "main");
        assert!(pr.head_sha.starts_with("e09354a7"));
        assert_eq!(pr.merge_state, "BLOCKED");
        let by: std::collections::HashMap<_, _> = pr
            .checks
            .iter()
            .map(|c| (c.name.as_str(), c.status.as_str()))
            .collect();
        assert_eq!(by["frontend"], "failure");
        assert_eq!(by["visual"], "pending");
        assert_eq!(by["backend"], "success");
        assert_eq!(
            by["lint/external"], "success",
            "StatusContext maps by state"
        );
        assert_eq!(pr.ci, "red");
        assert_eq!(pr.comments.len(), 2);
        assert!(pr.comments.iter().all(|c| c.bot), "reviewer-bot is a bot");
        assert!(pr.comments.iter().any(|c| c.kind == "review"));
        assert!(pr.comments.iter().any(|c| c.kind == "issue"));
    }

    #[test]
    fn ci_word_from_checks() {
        let c = |st: &str| PrCheck {
            name: "x".into(),
            status: st.into(),
            url: None,
        };
        assert_eq!(ci_of(&[]), "none");
        assert_eq!(ci_of(&[c("success"), c("skipped")]), "green");
        assert_eq!(ci_of(&[c("success"), c("pending")]), "pending");
        assert_eq!(ci_of(&[c("pending"), c("failure")]), "red");
    }

    /// REST inline comments keep path and line — what the auto-fix prompt
    /// points the agent at — oldest first, bodies capped.
    #[test]
    fn parses_inline_comments_with_path_and_line() {
        let cs = parse_inline_comments(&fixture("gh-2.46-pr-comments.json"));
        assert_eq!(cs.len(), 3);
        assert!(cs.iter().all(|c| c.kind == "inline" && c.bot));
        assert!(cs.iter().all(|c| c.path.is_some() && c.line.is_some()));
        assert!(cs
            .iter()
            .all(|c| c.body.chars().count() <= COMMENT_BODY_CAP + 1));
        assert!(cs.windows(2).all(|w| w[0].created_at <= w[1].created_at));
        let long = json!([{ "id": 1, "body": "x".repeat(2000), "user": {"login": "u"}, "created_at": "2026-01-01T00:00:00Z" }]);
        let one = parse_inline_comments(&long);
        assert_eq!(one[0].body.chars().count(), COMMENT_BODY_CAP + 1);
        assert!(one[0].body.ends_with('…'));
    }

    /// The gate in one table: a verified turn passes, a human-accepted
    /// verdict passes, an unverified turn and a commit the ledger never
    /// saw both hold the automatic merge.
    #[test]
    fn evidence_binds_commits_to_turns_and_verdicts() {
        let events = vec![
            ev(
                1,
                "prompt",
                "term:t1",
                Some("turn-a"),
                Some("t1"),
                json!({}),
            ),
            ev(
                2,
                "turn_end",
                "term:t1",
                Some("turn-a"),
                Some("t1"),
                json!({}),
            ),
            ev(
                3,
                "verification",
                "term:t1",
                Some("turn-a"),
                Some("t1"),
                json!({"status": "verified"}),
            ),
            ev(
                4,
                "commit",
                "term:t1",
                Some("turn-a"),
                Some("t1"),
                json!({"sha": "aaa", "subject": "A"}),
            ),
            ev(
                5,
                "prompt",
                "term:t1",
                Some("turn-b"),
                Some("t1"),
                json!({}),
            ),
            ev(
                6,
                "verification",
                "term:t1",
                Some("turn-b"),
                Some("t1"),
                json!({"status": "unverified_claim"}),
            ),
            ev(
                7,
                "commit",
                "term:t1",
                Some("turn-b"),
                Some("t1"),
                json!({"sha": "bbb", "subject": "B"}),
            ),
            ev(
                8,
                "verification_decision",
                "term:t1",
                Some("turn-b"),
                Some("t1"),
                json!({"decision": "accept", "verificationSeq": 6}),
            ),
            ev(
                9,
                "prompt",
                "term:t2",
                Some("turn-c"),
                Some("t2"),
                json!({}),
            ),
            ev(
                10,
                "commit",
                "term:t2",
                Some("turn-c"),
                Some("t2"),
                json!({"sha": "ccc", "subject": "C"}),
            ),
        ];
        let commits = |shas: &[&str]| -> Vec<(String, String)> {
            shas.iter()
                .map(|s| (s.to_string(), s.to_uppercase()))
                .collect()
        };
        let e = evidence_for("feat/x", "main", &commits(&["aaa", "bbb"]), &events);
        assert!(e.all_verified, "verified + human-accepted = no word needed");
        assert_eq!((e.verified, e.accepted), (1, 1));
        assert_eq!(e.commits[0].term_id.as_deref(), Some("t1"));
        assert_eq!(e.commits[1].verification, "accepted");

        let e = evidence_for("feat/x", "main", &commits(&["aaa", "ccc"]), &events);
        assert!(!e.all_verified, "a turn without a verdict holds the gate");
        assert_eq!(e.unverified, 1);
        assert_eq!(e.commits[1].verification, "unverified");

        let e = evidence_for("feat/x", "main", &commits(&["aaa", "ddd"]), &events);
        assert!(
            !e.all_verified,
            "a commit the ledger never saw holds the gate"
        );
        assert_eq!(e.unbound, 1);
        assert_eq!(e.commits[1].verification, "unbound");
        assert!(e.commits[1].case_id.is_none());

        let e = evidence_for("feat/x", "main", &[], &events);
        assert!(!e.all_verified, "nothing to merge is not verified");
    }

    #[test]
    fn link_payload_names_pr_and_commits() {
        let ov = PrOverview {
            branch: "feat/x".into(),
            pr: Some(parse_pr_view(&fixture("gh-2.46-pr-view.json"))),
            evidence: evidence_for("feat/x", "main", &[("aaa".into(), "A".into())], &[]),
            gh: "ok".into(),
            error: None,
        };
        let p = link_payload(&ov);
        assert_eq!(p["number"], 269);
        assert_eq!(p["ci"], "red");
        assert_eq!(p["commits"][0]["verification"], "unbound");
        assert_eq!(p["allVerified"], false);
    }
}
