//! Memory injection endpoint (E1 of the knowledge flywheel).
//!
//! A localhost-only HTTP listener that the UserPromptSubmit bridge scripts
//! call to fetch the memories/skills most relevant to the prompt being
//! submitted. The bridge merges the answer into the hook's
//! `hookSpecificOutput.additionalContext`, so the agent starts every turn
//! already knowing what this workspace has learned — for BOTH engines, since
//! Codex consumes the same hook schema.
//!
//! Hard rules, in order of importance:
//! - Never slow a prompt down: every miss (toggle off, model not downloaded,
//!   no index, malformed request) answers "nothing" immediately. The bridge
//!   additionally guards with its own short timeout.
//! - Never do heavy work on the prompt path: strictly read-only over the
//!   existing semantic index; no model downloads, no reindexing.
//! - Never hide an injection: every served context is recorded and emitted to
//!   the GUI (`inject://done`) so the user can always see what the agent was
//!   fed.
//! - Loopback only (POLITICA: nothing listens outside 127.0.0.1).

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::error::{AppError, AppResult};
use crate::services::corpus_feedback;
use crate::services::embedding_service::{model_ready, CandleEmbedder, Embedder};
use crate::services::projects_service;
use crate::services::semantic_index::{self, SearchHit};
use crate::services::work_profile;

/// How many hits may be injected into one prompt. Deliberately small: the
/// point is a nudge in the right direction, not a memory dump.
const MAX_HITS: usize = 2;
/// Candidates fetched before outcome adjustment (E2): exclusions and nudges
/// are applied over this pool, THEN the top MAX_HITS survive — an excluded
/// doc must not occupy a slot a useful one could take.
const CANDIDATE_POOL: usize = 8;
/// Cosine floor below which a hit is noise, not relevance. e5 scores cluster
/// high; 0.74 keeps genuinely-related notes and drops topic-adjacent ones.
const SCORE_THRESHOLD: f32 = 0.74;
/// Cap on the injected block — context budget is the user's, not ours.
const MAX_CONTEXT_CHARS: usize = 1200;
/// Requests are one prompt + a couple of paths; anything bigger is not ours.
const MAX_BODY_BYTES: usize = 64 * 1024;
/// Socket read timeout: a stuck local client must not wedge the accept loop.
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// Recent injections kept for the GUI ("what was the agent fed lately?").
const RECENT_CAP: usize = 20;

// ---------------------------------------------------------------------------
// Settings: per-project on/off toggle. Default ON — the flywheel only spins
// if it runs by default — so the file stores the roots that opted OUT.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InjectSettings {
    #[serde(default)]
    disabled_roots: Vec<String>,
}

fn settings_path() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("memory-inject.json"))
}

fn load_settings() -> InjectSettings {
    let Ok(path) = settings_path() else {
        return InjectSettings::default();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return InjectSettings::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_settings(s: &InjectSettings) -> AppResult<()> {
    let path = settings_path()?;
    let raw = serde_json::to_string_pretty(s).map_err(|e| AppError::Other(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn is_enabled(project_root: &str) -> bool {
    !load_settings()
        .disabled_roots
        .iter()
        .any(|r| r == project_root)
}

pub fn set_enabled(project_root: &str, enabled: bool) -> AppResult<()> {
    let mut s = load_settings();
    s.disabled_roots.retain(|r| r != project_root);
    if !enabled {
        s.disabled_roots.push(project_root.to_string());
    }
    save_settings(&s)
}

// ---------------------------------------------------------------------------
// Core: prompt → relevant context block.
// ---------------------------------------------------------------------------

/// Map a session cwd onto a known project root (longest match wins, so a
/// worktree under a project resolves to the project). Falls back to the cwd
/// itself — a project outside the recents list may still have an index.
fn resolve_project_root(cwd: &str, known_roots: &[String]) -> String {
    let norm = cwd.trim_end_matches(['/', '\\']);
    known_roots
        .iter()
        .map(|r| r.trim_end_matches(['/', '\\']))
        .filter(|r| {
            !r.is_empty()
                && (norm == *r
                    || norm
                        .strip_prefix(r)
                        .is_some_and(|rest| rest.starts_with(['/', '\\'])))
        })
        .max_by_key(|r| r.len())
        .unwrap_or(norm)
        .to_string()
}

/// Render kept hits as the context block the agent will read. The stale-memory
/// caveat is part of the format on purpose: retrieved notes are background,
/// not authority.
fn build_context(hits: &[SearchHit]) -> Option<String> {
    if hits.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Background notes retrieved from this workspace's Agent Console memory \
         (may be stale — verify against the current code before relying on them):",
    );
    for h in hits {
        let line = format!("\n- [{}] {}: {}", h.kind, h.title, h.snippet);
        if out.chars().count() + line.chars().count() > MAX_CONTEXT_CHARS {
            break;
        }
        out.push_str(&line);
    }
    Some(out)
}

/// What one served injection looked like — kept for the GUI and emitted as
/// the `inject://done` event payload.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectionRecord {
    pub ts_ms: u64,
    pub project_root: String,
    /// Head of the prompt that triggered the injection (identification, not
    /// surveillance — 80 chars is enough to recognize "which prompt was that").
    pub prompt_head: String,
    /// Terminal session the prompt came from, when the bridge knew it.
    pub term_id: Option<String>,
    pub hits: Vec<InjectedHit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectedHit {
    /// Corpus doc id ("memory:<file>", "skill:…") — what verdicts attach to.
    pub id: String,
    pub title: String,
    pub kind: String,
    pub score: f32,
}

static RECENT: Mutex<VecDeque<InjectionRecord>> = Mutex::new(VecDeque::new());

/// Terminal sessions that already received the work profile (E3). In-memory
/// on purpose: an app restart forgets — and re-serving once per app run is
/// the safe direction, since resumed sessions may have compacted it away.
static PROFILE_SERVED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// The profile block for this terminal, once per app run. None when the
/// terminal is unknown (can't dedupe → don't inject), already served, or the
/// profile has no meaningful content.
fn profile_block_for(term_id: &Option<String>) -> Option<String> {
    let term = term_id.as_deref()?;
    let excerpt = work_profile::injectable_excerpt()?;
    let mut guard = PROFILE_SERVED.lock().ok()?;
    let served = guard.get_or_insert_with(std::collections::HashSet::new);
    if !served.insert(term.to_string()) {
        return None;
    }
    Some(format!(
        "How this user works (their self-maintained Agent Console profile —          follow it unless they say otherwise):
{excerpt}"
    ))
}

#[cfg(test)]
fn reset_profile_served() {
    if let Ok(mut g) = PROFILE_SERVED.lock() {
        *g = None;
    }
}

pub fn recent() -> Vec<InjectionRecord> {
    RECENT
        .lock()
        .map(|q| q.iter().cloned().collect())
        .unwrap_or_default()
}

fn remember(record: InjectionRecord) {
    if let Ok(mut q) = RECENT.lock() {
        q.push_front(record);
        q.truncate(RECENT_CAP);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn head(text: &str, cap: usize) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut: String = clean.chars().take(cap).collect();
    if clean.chars().count() > cap {
        cut.push('…');
    }
    cut
}

/// Answer a prompt against a project's index: search, filter by score, build
/// the block. Pure over its inputs (embedder injected) so tests never need
/// the real model.
fn answer(
    project_root: &str,
    prompt: &str,
    embedder: &mut dyn Embedder,
) -> AppResult<(Vec<SearchHit>, Option<String>)> {
    let stats = corpus_feedback::stats(project_root);
    let mut hits = semantic_index::search(project_root, prompt, CANDIDATE_POOL, embedder)?;
    // Outcome signals (E2), injection-path only: drop docs the user has
    // repeatedly voted useless, nudge the rest by their verdict history —
    // bounded, so semantics still dominate (see corpus_feedback).
    hits.retain(|h| {
        !stats
            .get(&h.id)
            .map(corpus_feedback::DocStats::excluded)
            .unwrap_or(false)
    });
    for h in &mut hits {
        if let Some(st) = stats.get(&h.id) {
            h.score += st.nudge();
        }
    }
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(MAX_HITS);
    let kept: Vec<SearchHit> = hits
        .into_iter()
        .filter(|h| h.score >= SCORE_THRESHOLD)
        .collect();
    let context = build_context(&kept);
    Ok((kept, context))
}

// ---------------------------------------------------------------------------
// HTTP plumbing. Hand-rolled on purpose: one POST route, loopback only, a
// controlled client (our own bridge script) — a full HTTP dependency would be
// more audit surface than parser.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InjectRequest {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    cwd: String,
    /// Terminal session that submitted the prompt (AGENT_CONSOLE_TERM_ID) —
    /// lets the GUI pin the injection chip to the exact session.
    #[serde(default)]
    term_id: Option<String>,
}

fn port_file_path() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("inject-port.json"))
}

fn write_port_file(port: u16) -> AppResult<()> {
    let path = port_file_path()?;
    let raw = serde_json::json!({ "port": port, "pid": std::process::id() }).to_string();
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Read one request: request line + headers (Content-Length is the only one
/// we care about), then exactly the body. Returns the body for POST /inject;
/// anything else is a 404-worth of None.
fn read_request(stream: &mut TcpStream) -> Option<String> {
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let is_inject_post = {
        let mut parts = request_line.split_whitespace();
        parts.next() == Some("POST") && parts.next().is_some_and(|p| p.starts_with("/inject"))
    };
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
            .map(str::trim)
            .and_then(|v| v.parse::<usize>().ok())
        {
            content_length = v;
        }
    }
    if !is_inject_post || content_length == 0 || content_length > MAX_BODY_BYTES {
        return None;
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).ok()?;
    String::from_utf8(body).ok()
}

fn respond_json(stream: &mut TcpStream, body: &str) {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
}

/// The empty answer — what every gate and every error collapses to.
const NOTHING: &str = "{\"context\":null}";

fn serve_connection(stream: &mut TcpStream, app: &tauri::AppHandle) {
    let Some(body) = read_request(stream) else {
        respond_json(stream, NOTHING);
        return;
    };
    let Ok(req) = serde_json::from_str::<InjectRequest>(&body) else {
        respond_json(stream, NOTHING);
        return;
    };
    match handle_and_render(&req.prompt, &req.cwd, req.term_id) {
        Some((record, context)) => {
            let payload =
                serde_json::json!({ "context": context, "hits": record.hits }).to_string();
            remember(record.clone());
            let _ = app.emit("inject://done", &record);
            respond_json(stream, &payload);
        }
        None => respond_json(stream, NOTHING),
    }
}

/// Full request handling: cheap local gates first (empty input, toggle,
/// model on disk), then one search. `None` — "inject nothing" — is the only
/// failure mode this endpoint has.
fn handle_and_render(
    prompt: &str,
    cwd: &str,
    term_id: Option<String>,
) -> Option<(InjectionRecord, String)> {
    if prompt.trim().is_empty() || cwd.trim().is_empty() {
        return None;
    }
    let roots: Vec<String> = projects_service::load()
        .into_iter()
        .map(|p| p.path)
        .collect();
    let root = resolve_project_root(cwd, &roots);
    if !is_enabled(&root) {
        return None;
    }
    let mut blocks: Vec<String> = Vec::new();
    let mut hits: Vec<InjectedHit> = Vec::new();
    // The work profile (E3) needs no embeddings — it serves even before the
    // model is downloaded, once per terminal per app run.
    if let Some(profile) = profile_block_for(&term_id) {
        blocks.push(profile);
        hits.push(InjectedHit {
            id: "profile".into(),
            title: "work profile".into(),
            kind: "profile".into(),
            score: 1.0,
        });
    }
    // Semantic retrieval (E1/E2) only when the model is already local.
    if model_ready() {
        if let Ok((kept, Some(context))) = answer(&root, prompt, &mut CandleEmbedder::new()) {
            hits.extend(kept.iter().map(|h| InjectedHit {
                id: h.id.clone(),
                title: h.title.clone(),
                kind: h.kind.clone(),
                score: h.score,
            }));
            blocks.push(context);
        }
    }
    if blocks.is_empty() {
        return None;
    }
    let record = InjectionRecord {
        ts_ms: now_ms(),
        project_root: root,
        prompt_head: head(prompt, 80),
        term_id,
        hits,
    };
    // Durable injection log (E4): one line per served injection, feeding the
    // 30-day activity curve. Best-effort like everything on this path.
    crate::services::flywheel::record_injection(
        &record.project_root,
        record.ts_ms,
        record.hits.len() as u32,
    );
    // Passive usage signal (E2): count corpus injections. The profile is not
    // a corpus doc — verdicts don't apply to it — so it stays out.
    corpus_feedback::record_injected(
        &record.project_root,
        &record
            .hits
            .iter()
            .filter(|h| h.kind != "profile")
            .map(|h| h.id.clone())
            .collect::<Vec<_>>(),
        record.ts_ms,
    );
    Some((record, blocks.join("\n\n")))
}

/// Bind the loopback listener on an ephemeral port, announce it in the port
/// file, and serve forever on a background thread. Failure to start is loud
/// in the log but never fatal to the app — injection is an enhancement.
pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("inject: cannot bind loopback listener: {e}");
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(a) => a.port(),
            Err(e) => {
                eprintln!("inject: no local addr: {e}");
                return;
            }
        };
        if let Err(e) = write_port_file(port) {
            eprintln!("inject: cannot write port file: {e}");
            return;
        }
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            serve_connection(&mut stream, &app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_cwd_to_longest_known_root() {
        let roots = vec![
            "/home/u/work".to_string(),
            "/home/u/work/app".to_string(),
            "/home/u/other".to_string(),
        ];
        // Exact match, subdir, worktree under the deeper root.
        assert_eq!(
            resolve_project_root("/home/u/work/app", &roots),
            "/home/u/work/app"
        );
        assert_eq!(
            resolve_project_root("/home/u/work/app/.worktrees/f1", &roots),
            "/home/u/work/app"
        );
        assert_eq!(
            resolve_project_root("/home/u/work/lib", &roots),
            "/home/u/work"
        );
        // Sibling with a shared name prefix must NOT match ("/home/u/work2").
        assert_eq!(
            resolve_project_root("/home/u/work2", &roots),
            "/home/u/work2"
        );
        // Unknown cwd falls back to itself (trailing slash normalized).
        assert_eq!(resolve_project_root("/tmp/x/", &roots), "/tmp/x");
    }

    #[test]
    fn context_block_labels_hits_and_respects_the_cap() {
        let hit = |title: &str, snippet: &str| SearchHit {
            id: format!("memory:{title}"),
            kind: "memory".into(),
            title: title.into(),
            snippet: snippet.into(),
            score: 0.9,
        };
        assert_eq!(build_context(&[]), None);
        let block = build_context(&[hit("release lessons", "verify assets before announcing")])
            .expect("some");
        assert!(block.contains("may be stale"), "{block}");
        assert!(block.contains("[memory] release lessons:"), "{block}");
        // A hit that would blow the cap is dropped, not truncated mid-line.
        let long = "x".repeat(MAX_CONTEXT_CHARS);
        let block = build_context(&[hit("a", "short"), hit("b", &long)]).expect("some");
        assert!(block.contains("[memory] a"), "{block}");
        assert!(!block.contains(&long), "oversized hit must be dropped");
        assert!(block.chars().count() <= MAX_CONTEXT_CHARS + 80);
    }

    #[test]
    fn answer_filters_below_threshold_with_a_fake_embedder() {
        // Serial with other env-mutating tests.
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-inject-test-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        // Orthogonal fake: docs embed to axis vectors, the query matches doc0.
        struct Fake;
        impl Embedder for Fake {
            fn embed_passages(&mut self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
                Ok(texts
                    .iter()
                    .map(|t| {
                        if t.contains("release") {
                            vec![1.0, 0.0]
                        } else {
                            vec![0.0, 1.0]
                        }
                    })
                    .collect())
            }
            fn embed_query(&mut self, _text: &str) -> AppResult<Vec<f32>> {
                Ok(vec![1.0, 0.0])
            }
        }

        let root = "/proj/inject-answer";
        let docs = vec![
            crate::services::semantic_index::SourceDoc {
                id: "memory:release.md".into(),
                kind: "memory".into(),
                title: "release lessons".into(),
                text: "release pipeline lessons".into(),
            },
            crate::services::semantic_index::SourceDoc {
                id: "memory:other.md".into(),
                kind: "memory".into(),
                title: "unrelated".into(),
                text: "completely different topic".into(),
            },
        ];
        semantic_index::reindex(root, &docs, &mut Fake).unwrap();

        let (kept, context) = answer(root, "how do we release?", &mut Fake).unwrap();
        assert_eq!(kept.len(), 1, "orthogonal doc must fall below threshold");
        assert_eq!(kept[0].title, "release lessons");
        assert!(context.unwrap().contains("release lessons"));

        // E2 — exclusion: three unhelpful verdicts silence the doc entirely…
        for _ in 0..3 {
            crate::services::corpus_feedback::set_verdict(root, "memory:release.md", false)
                .unwrap();
        }
        let (kept, context) = answer(root, "how do we release?", &mut Fake).unwrap();
        assert!(kept.is_empty(), "excluded doc must not be injected");
        assert!(context.is_none());
        // …and rehabilitation brings it straight back.
        crate::services::corpus_feedback::reset_verdicts(root, "memory:release.md").unwrap();
        let (kept, _) = answer(root, "how do we release?", &mut Fake).unwrap();
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn answer_nudge_reorders_near_ties_only() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-inject-nudge-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        // Two near-tied docs (cosine 1.0 vs ~0.996) and one clearly-worse
        // (~0.7 — inside the pool, far outside nudge range).
        struct Fake;
        impl Embedder for Fake {
            fn embed_passages(&mut self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
                Ok(texts
                    .iter()
                    .map(|t| {
                        if t.contains("alpha") {
                            vec![1.0, 0.0]
                        } else if t.contains("beta") {
                            vec![0.996, 0.0893]
                        } else {
                            vec![0.7, 0.714]
                        }
                    })
                    .collect())
            }
            fn embed_query(&mut self, _text: &str) -> AppResult<Vec<f32>> {
                Ok(vec![1.0, 0.0])
            }
        }

        let root = "/proj/inject-nudge";
        let doc = |id: &str, text: &str| crate::services::semantic_index::SourceDoc {
            id: id.into(),
            kind: "memory".into(),
            title: id.into(),
            text: text.into(),
        };
        let docs = vec![
            doc("memory:alpha.md", "alpha"),
            doc("memory:beta.md", "beta"),
            doc("memory:far.md", "far away"),
        ];
        semantic_index::reindex(root, &docs, &mut Fake).unwrap();

        // Baseline: alpha wins the near-tie.
        let (kept, _) = answer(root, "q", &mut Fake).unwrap();
        assert_eq!(kept[0].id, "memory:alpha.md");

        // Two helpful votes on beta (+0.006) flip the ~0.004 gap…
        crate::services::corpus_feedback::set_verdict(root, "memory:beta.md", true).unwrap();
        crate::services::corpus_feedback::set_verdict(root, "memory:beta.md", true).unwrap();
        let (kept, _) = answer(root, "q", &mut Fake).unwrap();
        assert_eq!(kept[0].id, "memory:beta.md", "nudge breaks the near-tie");

        // …but even a saturated nudge can never lift the clearly-worse doc
        // (0.7 + 0.03 is far from both the tie and the threshold).
        for _ in 0..20 {
            crate::services::corpus_feedback::set_verdict(root, "memory:far.md", true).unwrap();
        }
        let (kept, _) = answer(root, "q", &mut Fake).unwrap();
        assert!(
            kept.iter().all(|h| h.id != "memory:far.md"),
            "bounded nudge must never overturn a clear semantic gap"
        );
    }

    #[test]
    fn profile_injects_once_per_terminal_and_needs_real_content() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-inject-profile-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
        // Isolate the cache too: model_ready() must be deterministically false
        // so this test proves the profile ships WITHOUT the embedding model.
        std::env::set_var("XDG_CACHE_HOME", base.join("cache"));
        reset_profile_served();

        crate::services::work_profile::set("## Conventions\nCommit in English.\n").unwrap();

        let (record, ctx) = handle_and_render(
            "a prompt long enough to qualify",
            "/proj/p",
            Some("t1".into()),
        )
        .expect("first prompt of t1 gets the profile");
        assert_eq!(record.hits.len(), 1);
        assert_eq!(record.hits[0].kind, "profile");
        assert!(ctx.contains("How this user works"), "{ctx}");
        assert!(ctx.contains("Commit in English."), "{ctx}");

        assert!(
            handle_and_render(
                "another qualifying prompt here",
                "/proj/p",
                Some("t1".into())
            )
            .is_none(),
            "same terminal: profile already served, nothing else to inject"
        );
        assert!(
            handle_and_render(
                "another qualifying prompt here",
                "/proj/p",
                Some("t2".into())
            )
            .is_some(),
            "a different terminal gets its own copy"
        );
        assert!(
            handle_and_render("no terminal id at all here", "/proj/p", None).is_none(),
            "unknown terminal: can't dedupe, don't inject"
        );

        // Template-only profile injects nothing at all.
        reset_profile_served();
        crate::services::work_profile::set(crate::services::work_profile::PROFILE_TEMPLATE)
            .unwrap();
        assert!(handle_and_render(
            "a prompt long enough to qualify",
            "/proj/p",
            Some("t9".into())
        )
        .is_none());
    }

    #[test]
    fn toggle_default_on_and_roundtrip() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-inject-settings-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let root = "/proj/toggle";
        assert!(is_enabled(root), "default must be ON");
        set_enabled(root, false).unwrap();
        assert!(!is_enabled(root));
        assert!(is_enabled("/proj/other"), "off is per-project");
        set_enabled(root, true).unwrap();
        assert!(is_enabled(root));
    }

    #[test]
    fn read_request_extracts_post_body_and_rejects_the_rest() {
        // Loopback pair so the parser is exercised over a real TcpStream.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();

        let send = |raw: &str| -> Option<String> {
            let raw = raw.to_string();
            let t = std::thread::spawn(move || {
                let mut c = TcpStream::connect(addr).unwrap();
                c.write_all(raw.as_bytes()).unwrap();
                // Keep the socket open until the server side is done reading.
                let mut buf = [0u8; 1];
                let _ = c.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = c.read(&mut buf);
            });
            let (mut s, _) = listener.accept().unwrap();
            let out = read_request(&mut s);
            respond_json(&mut s, NOTHING);
            t.join().unwrap();
            out
        };

        let body = r#"{"prompt":"p","cwd":"/x"}"#;
        let good = format!(
            "POST /inject HTTP/1.1\r\nHost: l\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        assert_eq!(send(&good).as_deref(), Some(body));

        // Wrong path, wrong method, missing length: all read as None.
        assert_eq!(
            send("POST /other HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}"),
            None
        );
        assert_eq!(send("GET /inject HTTP/1.1\r\n\r\n"), None);
        assert_eq!(send("POST /inject HTTP/1.1\r\n\r\n"), None);
    }
}
