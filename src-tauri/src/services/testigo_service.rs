use std::collections::HashMap;
use std::fs::{self, OpenOptions};

use std::io::Write;
use std::path::{Path, PathBuf};
use parking_lot::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

/// One link in a project's Testigo chain: the durable, hash-chained record of
/// something that happened between the human and the agent (a prompt, an
/// approval, a snapshot, a turn boundary).
///
/// Unlike the activity ledger (which is a trimmed substrate for learning mode),
/// this ledger is evidence: append-only, never trimmed, and each event carries
/// `hash = sha256(serialization with hash="")` chained through `prev_hash`, so
/// after-the-fact edits are detectable (`verify`). Tamper-EVIDENT, not
/// tamper-proof: anyone with disk access can rewrite the whole chain — the
/// claim is "this file is internally consistent", not "this file is signed".
/// Signing happens at packet export (F3).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofEvent {
    pub seq: u64,
    /// Epoch milliseconds (from the hook payload when present, else 0).
    pub ts: i64,
    /// The intent thread this event belongs to: "jira:<KEY>" when the session
    /// was seeded from a ticket, else "term:<termId>", else "unbound".
    pub case_id: String,
    /// The turn (prompt → stop) this event happened inside, when known. The
    /// binding is heuristic — same termId between a prompt and its stop — not
    /// cryptographic; the protocol spec states this openly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// "prompt" | "turn_end" | "approval_request" | "approval_decision"
    /// | "snapshot" | "case_link"
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Who produced the event: "human" (prompt, decision), "agent" (tool
    /// request, turn end), "system" (snapshot, case link).
    pub actor: String,
    pub payload: Value,
    pub prev_hash: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub ok: bool,
    pub total: usize,
    /// Seq of the first event whose hash, chain link, or seq is inconsistent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broken_at_seq: Option<u64>,
    /// A crash-torn final line exists. Tolerated (the chain up to it is intact).
    pub torn_tail: bool,
}

/// Tool inputs can be arbitrarily large (file writes); the ledger keeps a
/// bounded preview so one approval can't balloon the evidence file.
const MAX_INPUT_BYTES: usize = 4096;

/// Per-project Testigo policy.
///
/// Defaults are deliberately asymmetric: `witness` on (the ledger lives
/// outside the repo — private, local, costless to keep), `repo_marks` OFF
/// (trailers and the anchor ref touch the project's git — in shared repos
/// that's a decision the owner must make, so it's opt-in per project).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestigoSettings {
    /// Record events into the local ledger at all.
    pub witness: bool,
    /// Stamp commit trailers (Testigo-Case/Testigo-Head) and pin the anchor
    /// ref in the project's checkout.
    pub repo_marks: bool,
    /// Request an RFC 3161 timestamp over the packet signature at export,
    /// from this TSA URL (spec §2.5). None = no timestamp (default) — the
    /// request sends the TSA a signature hash, so leaving the machine is the
    /// owner's call, like repo marks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_tsa: Option<String>,
}

impl Default for TestigoSettings {
    fn default() -> Self {
        Self {
            witness: true,
            repo_marks: false,
            timestamp_tsa: None,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    #[serde(default)]
    by_project: HashMap<String, TestigoSettings>,
}

/// Context of the turn currently open in a terminal: the id every in-turn
/// event attaches to, plus what turn_end needs to compute the pre/post diff.
#[derive(Debug, Clone)]
pub struct TurnState {
    pub turn_id: String,
    /// Snapshot of the working tree taken right after the prompt (the first
    /// snapshot of the turn) — the "before" side of the turn diff.
    pub pre_sha: Option<String>,
    /// Where the agent runs (worktree sessions differ from the project root).
    pub cwd: Option<String>,
}

#[derive(Default)]
struct Inner {
    /// project_root -> last (seq, hash); None = ledger empty. Lazily loaded
    /// from the file tail on first touch per project.
    tails: HashMap<String, Option<(u64, String)>>,
    /// term_id -> case_id override (from case_link events; survives restarts
    /// because ensure_tail rebuilds it from the ledger).
    cases: HashMap<String, String>,
    /// term_id -> currently open turn (in-memory only: a restart mid-turn
    /// loses attribution until the next prompt, which is acceptable).
    turns: HashMap<String, TurnState>,
    /// approval id -> (term_id, tool), so the decision event can name what it
    /// approved even though the respond() path only carries the id.
    approvals: HashMap<String, (Option<String>, Option<String>)>,
    /// Per-project policy, lazily loaded from testigo-settings.json.
    settings: Option<HashMap<String, TestigoSettings>>,
}

pub struct TestigoService {
    inner: Mutex<Inner>,
}

impl TestigoService {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console")
            .join("testigo");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// pub(crate): the export module reads raw ledger lines byte-exactly.
    pub(crate) fn ledger_path(project_root: &str) -> AppResult<PathBuf> {
        Ok(Self::dir()?.join(crate::services::persistence::project_file_key(project_root)))
    }

    fn settings_path() -> AppResult<PathBuf> {
        Ok(dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console")
            .join("testigo-settings.json"))
    }

    fn load_settings_file() -> SettingsFile {
        let Ok(path) = Self::settings_path() else {
            return SettingsFile::default();
        };
        let read = |p: &Path| -> Option<SettingsFile> {
            serde_json::from_str(&fs::read_to_string(p).ok()?).ok()
        };
        read(&path)
            .or_else(|| read(&path.with_extension("json.bak")))
            .unwrap_or_default()
    }

    fn settings_map(inner: &mut Inner) -> &mut HashMap<String, TestigoSettings> {
        inner
            .settings
            .get_or_insert_with(|| Self::load_settings_file().by_project)
    }

    /// This project's policy (defaults when never configured).
    pub fn settings(&self, project_root: &str) -> TestigoSettings {
        let mut inner = self.inner.lock();
        Self::settings_map(&mut inner)
            .get(project_root)
            .cloned()
            .unwrap_or_default()
    }

    /// Convenience for the anchor/trailer call sites.
    pub fn repo_marks(&self, project_root: &str) -> bool {
        self.settings(project_root).repo_marks
    }

    /// Persist this project's policy: load-before-save merge into the shared
    /// file, temp+rename atomic write, .bak of the previous version.
    pub fn set_settings(&self, project_root: &str, s: TestigoSettings) -> AppResult<TestigoSettings> {
        let mut inner = self.inner.lock();
        Self::settings_map(&mut inner).insert(project_root.to_string(), s.clone());
        let mut file = Self::load_settings_file();
        file.by_project.insert(project_root.to_string(), s.clone());
        drop(inner);
        let path = Self::settings_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(&file)
            .map_err(|e| AppError::Other(format!("serialize settings: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        if path.exists() {
            let _ = fs::copy(&path, path.with_extension("json.bak"));
        }
        fs::write(&tmp, body)?;
        fs::rename(&tmp, &path)?;
        Ok(s)
    }

    fn event_hash(ev: &ProofEvent) -> String {
        let mut unhashed = ev.clone();
        unhashed.hash = String::new();
        let bytes = serde_json::to_string(&unhashed).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(bytes.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Load the chain tail (and rebuild term→case bindings) from disk on the
    /// first touch of a project. Full read of the ledger; it happens once per
    /// project per app run, so an occasional large file costs one scan.
    ///
    /// A crash-torn FINAL line is self-healed here (truncated via temp+rename)
    /// — otherwise the next append would land after the garbage and verify
    /// would flag an intact-but-interrupted chain as broken. Unparseable lines
    /// anywhere else are NOT healed: that's tampering, verify's job to report.
    fn ensure_tail(inner: &mut Inner, project_root: &str) -> AppResult<()> {
        if inner.tails.contains_key(project_root) {
            return Ok(());
        }
        let path = Self::ledger_path(project_root)?;
        let mut tail: Option<(u64, String)> = None;
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
            let torn_tail = lines
                .last()
                .is_some_and(|l| serde_json::from_str::<ProofEvent>(l).is_err());
            for line in &lines {
                let Ok(ev) = serde_json::from_str::<ProofEvent>(line) else {
                    continue;
                };
                if ev.kind == "case_link" {
                    if let Some(term) = &ev.term_id {
                        inner.cases.insert(term.clone(), ev.case_id.clone());
                    }
                }
                tail = Some((ev.seq, ev.hash));
            }
            if torn_tail {
                let kept: String = lines[..lines.len() - 1]
                    .iter()
                    .map(|l| format!("{l}\n"))
                    .collect();
                let tmp = path.with_extension("jsonl.tmp");
                fs::write(&tmp, kept)?;
                fs::rename(&tmp, &path)?;
            }
        }
        inner.tails.insert(project_root.to_string(), tail);
        Ok(())
    }

    fn case_for(inner: &Inner, term_id: Option<&str>) -> String {
        match term_id {
            Some(t) => inner
                .cases
                .get(t)
                .cloned()
                .unwrap_or_else(|| format!("term:{t}")),
            None => "unbound".into(),
        }
    }

    /// Assign seq/prev_hash/hash and append. Called with the lock held so
    /// concurrent writers can't interleave and fork the chain — the whole
    /// point of this ledger is a single linear history per project.
    #[allow(clippy::too_many_arguments)]
    fn record(
        inner: &mut Inner,
        project_root: &str,
        ts: i64,
        case_id: String,
        turn_id: Option<String>,
        kind: &str,
        term_id: Option<String>,
        session_id: Option<String>,
        actor: &str,
        payload: Value,
    ) -> AppResult<ProofEvent> {
        // Per-project witness switch: every ledger write funnels through here,
        // so one check covers prompts, approvals, results, jobs and links.
        // Reads (list/verify/export of PAST evidence) are unaffected.
        if !Self::settings_map(inner)
            .get(project_root)
            .is_none_or(|s| s.witness)
        {
            return Err(AppError::Other("testigo: witnessing disabled for this project".into()));
        }
        Self::ensure_tail(inner, project_root)?;
        let tail = inner.tails.get(project_root).cloned().flatten();
        let (seq, prev_hash) = match tail {
            Some((s, h)) => (s + 1, h),
            None => (0, "genesis".to_string()),
        };
        let mut ev = ProofEvent {
            seq,
            ts,
            case_id,
            turn_id,
            kind: kind.into(),
            term_id,
            session_id,
            actor: actor.into(),
            payload,
            prev_hash,
            hash: String::new(),
        };
        ev.hash = Self::event_hash(&ev);

        let path = Self::ledger_path(project_root)?;
        let mut line = serde_json::to_string(&ev)
            .map_err(|e| AppError::Other(format!("serialize proof event: {e}")))?;
        line.push('\n');
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        f.write_all(line.as_bytes())?;
        drop(f);
        inner
            .tails
            .insert(project_root.to_string(), Some((ev.seq, ev.hash.clone())));
        Ok(ev)
    }

    /// A user prompt opens a new turn for its terminal (implicitly superseding
    /// any turn still open there — engines don't always emit Stop, e.g. on a
    /// killed session).
    #[allow(clippy::too_many_arguments)]
    pub fn on_prompt(
        &self,
        project_root: &str,
        ts: i64,
        term_id: Option<&str>,
        session_id: Option<&str>,
        prompt: Option<&str>,
        skill: Option<&str>,
        cwd: Option<&str>,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        Self::ensure_tail(&mut inner, project_root)?;
        let turn_id = uuid::Uuid::new_v4().to_string();
        if let Some(t) = term_id {
            inner.turns.insert(
                t.to_string(),
                TurnState {
                    turn_id: turn_id.clone(),
                    pre_sha: None,
                    cwd: cwd.map(String::from),
                },
            );
        }
        let case = Self::case_for(&inner, term_id);
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            Some(turn_id),
            "prompt",
            term_id.map(String::from),
            session_id.map(String::from),
            "human",
            json!({ "prompt": prompt, "skill": skill, "cwd": cwd }),
        )
    }

    /// Context of the turn currently open in `term_id`, if any — what the
    /// turn_end handler needs (pre snapshot + cwd) to compute the turn diff
    /// BEFORE closing the turn. Read-only; `on_turn_end` does the removal.
    pub fn peek_turn(&self, term_id: &str) -> Option<TurnState> {
        self.inner.lock().turns.get(term_id).cloned()
    }

    /// Close the turn with the caller-built result payload (pre/post snapshot
    /// shas and the files-changed diff, when the checkout is a git repo).
    pub fn on_turn_end(
        &self,
        project_root: &str,
        ts: i64,
        term_id: Option<&str>,
        session_id: Option<&str>,
        payload: Value,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        let turn_id = term_id.and_then(|t| inner.turns.remove(t)).map(|s| s.turn_id);
        let case = Self::case_for(&inner, term_id);
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "turn_end",
            term_id.map(String::from),
            session_id.map(String::from),
            "agent",
            payload,
        )
    }

    /// Record what one tool call produced inside the open turn. The hook
    /// already bounds the excerpt; `bounded_input` is a second guard.
    #[allow(clippy::too_many_arguments)]
    pub fn on_tool_result(
        &self,
        project_root: &str,
        ts: i64,
        term_id: Option<&str>,
        session_id: Option<&str>,
        tool: Option<&str>,
        excerpt: Option<&str>,
        truncated: bool,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        let turn_id = term_id.and_then(|t| inner.turns.get(t)).map(|s| s.turn_id.clone());
        let case = Self::case_for(&inner, term_id);
        let excerpt_v = bounded_input(json!(excerpt));
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "tool_result",
            term_id.map(String::from),
            session_id.map(String::from),
            "agent",
            json!({ "tool": tool, "excerpt": excerpt_v, "truncated": truncated }),
        )
    }

    /// Record a scheduler job run under its own "job:<id>" case — scheduled
    /// work is agentic action too, and its outcome belongs in the evidence.
    pub fn on_job_run(
        &self,
        project_root: &str,
        ts: i64,
        job_id: &str,
        job_name: &str,
        status: &str,
        summary: &str,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        Self::ensure_tail(&mut inner, project_root)?;
        Self::record(
            &mut inner,
            project_root,
            ts,
            format!("job:{job_id}"),
            None,
            "job_run",
            None,
            None,
            "system",
            json!({ "jobId": job_id, "jobName": job_name, "status": status, "summary": summary }),
        )
    }

    pub fn on_snapshot(
        &self,
        project_root: &str,
        ts: i64,
        term_id: Option<&str>,
        session_id: Option<&str>,
        commit_sha: &str,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        let turn_id = term_id.and_then(|t| {
            let state = inner.turns.get_mut(t)?;
            // The first snapshot of a turn (taken right after the prompt) is
            // the "before" side of the turn diff computed at turn_end.
            if state.pre_sha.is_none() {
                state.pre_sha = Some(commit_sha.to_string());
            }
            Some(state.turn_id.clone())
        });
        let case = Self::case_for(&inner, term_id);
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "snapshot",
            term_id.map(String::from),
            session_id.map(String::from),
            "system",
            json!({ "commitSha": commit_sha }),
        )
    }

    /// Record a PreToolUse approval request (the raw hook payload). Keeps an
    /// in-memory id→context map so the later decision can name the tool.
    pub fn on_approval_request(&self, project_root: &str, v: &Value) -> AppResult<ProofEvent> {
        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("");
        let tool = v.get("tool").and_then(|x| x.as_str()).map(String::from);
        let term_id = v.get("termId").and_then(|x| x.as_str()).map(String::from);
        let ts = v.get("ts").and_then(|x| x.as_i64()).unwrap_or(0);
        let input = bounded_input(v.get("input").cloned().unwrap_or(Value::Null));

        let mut inner = self.inner.lock();
        inner
            .approvals
            .insert(id.to_string(), (term_id.clone(), tool.clone()));
        let turn_id = term_id
            .as_deref()
            .and_then(|t| inner.turns.get(t))
            .map(|s| s.turn_id.clone());
        let case = Self::case_for(&inner, term_id.as_deref());
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "approval_request",
            term_id,
            None,
            "agent",
            json!({
                "approvalId": id,
                "tool": tool,
                "input": input,
                "cwd": v.get("cwd").and_then(|x| x.as_str()),
            }),
        )
    }

    /// Record the human's decision on an in-flight approval. This is the audit
    /// trail the raw hook files never had: they are polled and deleted, this
    /// line is forever.
    pub fn on_approval_decision(
        &self,
        project_root: &str,
        ts: i64,
        id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        // remove(): one decision closes one request. A post-restart decision
        // (empty map) still records, just without tool/term context.
        let (term_id, tool) = inner.approvals.remove(id).unwrap_or((None, None));
        let turn_id = term_id
            .as_deref()
            .and_then(|t| inner.turns.get(t))
            .map(|s| s.turn_id.clone());
        let case = Self::case_for(&inner, term_id.as_deref());
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "approval_decision",
            term_id,
            None,
            "human",
            json!({ "approvalId": id, "tool": tool, "decision": decision, "reason": reason }),
        )
    }

    /// Bind a terminal's events to a named case — today "jira:<KEY>" when a
    /// session is seeded from a ticket. Recorded as an event (not just state)
    /// so the binding is itself part of the evidence and survives restarts.
    pub fn link_case(
        &self,
        project_root: &str,
        ts: i64,
        term_id: &str,
        case_id: &str,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        Self::ensure_tail(&mut inner, project_root)?;
        inner.cases.insert(term_id.to_string(), case_id.to_string());
        Self::record(
            &mut inner,
            project_root,
            ts,
            case_id.to_string(),
            None,
            "case_link",
            Some(term_id.to_string()),
            None,
            "system",
            json!({}),
        )
    }

    /// The case that produced these files, per the ledger: the most recent
    /// turn_end (within `max_age_ms`) whose filesChanged intersects `files`.
    /// Used to stamp console-made commits with a `Testigo-Case:` trailer —
    /// attribution comes from recorded evidence (which turn touched which
    /// files), never from "whatever session is active".
    pub fn case_for_files(
        &self,
        project_root: &str,
        files: &[String],
        now_ms: i64,
        max_age_ms: i64,
    ) -> AppResult<Option<String>> {
        if files.is_empty() {
            return Ok(None);
        }
        let events = self.list(project_root, None, None)?;
        for ev in events.iter().rev() {
            if ev.kind != "turn_end" || now_ms - ev.ts > max_age_ms {
                continue;
            }
            let Some(changed) = ev.payload.get("filesChanged").and_then(|v| v.as_array()) else {
                continue;
            };
            let touched = changed
                .iter()
                .filter_map(|f| f.get("path").and_then(|p| p.as_str()))
                .any(|p| files.iter().any(|f| f == p));
            if touched {
                return Ok(Some(ev.case_id.clone()));
            }
        }
        Ok(None)
    }

    /// The ledger's current tail: (seq, hash) of the last event, None when
    /// empty. Cached; loads from disk on first touch.
    pub fn head(&self, project_root: &str) -> AppResult<Option<(u64, String)>> {
        let mut inner = self.inner.lock();
        Self::ensure_tail(&mut inner, project_root)?;
        Ok(inner.tails.get(project_root).cloned().flatten())
    }

    /// Events in chronological order, optionally filtered by case and capped
    /// at the most recent `limit`.
    pub fn list(
        &self,
        project_root: &str,
        case_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<ProofEvent>> {
        let _g = self.inner.lock();
        let path = Self::ledger_path(project_root)?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path)?;
        let mut events: Vec<ProofEvent> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<ProofEvent>(l).ok())
            .filter(|e| case_id.is_none_or(|c| e.case_id == c))
            .collect();
        if let Some(n) = limit {
            if events.len() > n {
                events = events.split_off(events.len() - n);
            }
        }
        Ok(events)
    }

    /// Walk the whole chain recomputing hashes and links. A torn final line
    /// (crash mid-append) is tolerated and reported; anything else
    /// inconsistent marks the chain broken at that seq.
    pub fn verify(&self, project_root: &str) -> AppResult<VerifyReport> {
        let _g = self.inner.lock();
        let path = Self::ledger_path(project_root)?;
        if !path.exists() {
            return Ok(VerifyReport {
                ok: true,
                total: 0,
                broken_at_seq: None,
                torn_tail: false,
            });
        }
        let content = fs::read_to_string(&path)?;
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut prev: Option<(u64, String)> = None;
        let mut total = 0usize;
        for (i, line) in lines.iter().enumerate() {
            let parsed = serde_json::from_str::<ProofEvent>(line);
            let Ok(ev) = parsed else {
                if i == lines.len() - 1 {
                    return Ok(VerifyReport {
                        ok: true,
                        total,
                        broken_at_seq: None,
                        torn_tail: true,
                    });
                }
                return Ok(VerifyReport {
                    ok: false,
                    total,
                    broken_at_seq: Some(prev.map(|(s, _)| s + 1).unwrap_or(0)),
                    torn_tail: false,
                });
            };
            let expected_prev = prev
                .as_ref()
                .map(|(_, h)| h.clone())
                .unwrap_or_else(|| "genesis".into());
            let expected_seq = prev.as_ref().map(|(s, _)| s + 1).unwrap_or(0);
            let recomputed = Self::event_hash(&ev);
            if ev.prev_hash != expected_prev || ev.seq != expected_seq || ev.hash != recomputed {
                return Ok(VerifyReport {
                    ok: false,
                    total,
                    broken_at_seq: Some(ev.seq),
                    torn_tail: false,
                });
            }
            prev = Some((ev.seq, ev.hash));
            total += 1;
        }
        Ok(VerifyReport {
            ok: true,
            total,
            broken_at_seq: None,
            torn_tail: false,
        })
    }
}

/// Anchor the ledger head in the checkout's git object store: a blob with
/// {seq, hash, ts} pinned at `refs/agent-console/testigo-head`. Rewriting the
/// ledger consistently now also requires rewriting this ref — and the
/// `Testigo-Head:` commit trailers that carry the same value into pushed
/// history. Stronger tamper-EVIDENCE, still not tamper-proof: an actor with
/// full local control can rewrite both; distribution (pushes, clones) is what
/// makes the anchor stick. No-op when `repo` is not a git checkout.
pub fn anchor_head(repo: &Path, seq: u64, hash: &str, ts: i64) -> AppResult<()> {
    use crate::services::proc;
    let body = format!("{{\"seq\":{seq},\"hash\":\"{hash}\",\"ts\":{ts}}}\n");
    let tmp = std::env::temp_dir().join(format!("testigo-anchor-{}-{seq}", std::process::id()));
    fs::write(&tmp, body)?;
    let out = proc::command("git")
        .args(["hash-object", "-w", &tmp.to_string_lossy()])
        .current_dir(repo)
        .output();
    let _ = fs::remove_file(&tmp);
    let out = out?;
    if !out.status.success() {
        // Not a git repo (or object write failed): anchoring is best-effort
        // by design — the ledger itself is unaffected.
        return Ok(());
    }
    let oid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let _ = proc::command("git")
        .args(["update-ref", "refs/agent-console/testigo-head", &oid])
        .current_dir(repo)
        .output()?;
    Ok(())
}

/// Bound a tool input to MAX_INPUT_BYTES of serialized JSON, replacing it with
/// a marked preview when it's larger — approvals must never balloon the ledger.
fn bounded_input(input: Value) -> Value {
    let s = input.to_string();
    if s.len() <= MAX_INPUT_BYTES {
        return input;
    }
    let mut end = MAX_INPUT_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    json!({ "truncated": true, "preview": &s[..end] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// One test fn on purpose (mutates process-global XDG_DATA_HOME): exercises
    /// the full chain — prompt opens a turn, approvals attach to it, turn_end
    /// closes it, case_link rebinds, list filters, verify detects tampering
    /// and tolerates a torn tail.
    #[test]
    fn chain_records_links_and_verifies() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-testigo-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = TestigoService::new();
        let root = "/proj/a";

        // Fresh project: empty list, verify ok on missing file.
        assert!(svc.list(root, None, None).unwrap().is_empty());
        assert!(svc.verify(root).unwrap().ok);

        // Ticket seeding binds the terminal to a jira case BEFORE any prompt.
        svc.link_case(root, 1, "t1", "jira:FIXY-1").unwrap();

        // Prompt opens a turn under that case.
        let p = svc
            .on_prompt(root, 2, Some("t1"), Some("s1"), Some("do it"), None, None)
            .unwrap();
        assert_eq!(p.case_id, "jira:FIXY-1");
        let turn = p.turn_id.clone().unwrap();

        // Approval request + decision inherit case AND turn; decision knows the tool.
        let req = serde_json::json!({
            "id": "ap1", "ts": 3, "tool": "write_file",
            "input": { "path": "x" }, "cwd": "/proj/a", "termId": "t1"
        });
        let r = svc.on_approval_request(root, &req).unwrap();
        assert_eq!(r.turn_id.as_deref(), Some(turn.as_str()));
        let d = svc
            .on_approval_decision(root, 4, "ap1", "allow", Some("looks safe"))
            .unwrap();
        assert_eq!(d.case_id, "jira:FIXY-1");
        assert_eq!(d.payload["tool"], "write_file");
        assert_eq!(d.actor, "human");

        // The first snapshot of the turn becomes the "before" side of the
        // turn diff (peek_turn exposes it to the turn_end handler).
        let s = svc.on_snapshot(root, 5, Some("t1"), Some("s1"), "abc123").unwrap();
        assert_eq!(s.turn_id.as_deref(), Some(turn.as_str()));
        let peeked = svc.peek_turn("t1").unwrap();
        assert_eq!(peeked.pre_sha.as_deref(), Some("abc123"));

        // A tool result lands inside the same turn, excerpt preserved.
        let tr = svc
            .on_tool_result(root, 6, Some("t1"), Some("s1"), Some("Bash"), Some("ok\n"), false)
            .unwrap();
        assert_eq!(tr.turn_id.as_deref(), Some(turn.as_str()));
        assert_eq!(tr.actor, "agent");

        // The turn closes carrying its id and the caller-built result payload.
        let e = svc
            .on_turn_end(
                root,
                7,
                Some("t1"),
                Some("s1"),
                serde_json::json!({
                    "preSha": "abc123",
                    "postSha": "def456",
                    "filesChanged": [{ "status": "A", "path": "src/x.rs" }],
                }),
            )
            .unwrap();
        assert_eq!(e.turn_id.as_deref(), Some(turn.as_str()));
        assert_eq!(e.payload["postSha"], "def456");
        assert!(svc.peek_turn("t1").is_none(), "turn closed");

        // Commit-trailer attribution: the ledger names the case that touched
        // a staged file — recency-bounded, evidence-based, no active-session
        // guessing.
        let hit = svc
            .case_for_files(root, &["src/x.rs".into()], 10, 1000)
            .unwrap();
        assert_eq!(hit.as_deref(), Some("jira:FIXY-1"));
        assert!(svc
            .case_for_files(root, &["unrelated.rs".into()], 10, 1000)
            .unwrap()
            .is_none());
        assert!(
            svc.case_for_files(root, &["src/x.rs".into()], 10_000, 100)
                .unwrap()
                .is_none(),
            "stale turns don't stamp"
        );

        // An unlinked terminal falls back to a term case; projects isolate.
        let other = svc
            .on_prompt(root, 8, Some("t2"), None, Some("hi"), None, None)
            .unwrap();
        assert_eq!(other.case_id, "term:t2");
        assert!(svc.list("/proj/b", None, None).unwrap().is_empty());

        // Scheduler runs chain in under their own job case.
        let jr = svc.on_job_run(root, 9, "j1", "nightly", "ok", "all good").unwrap();
        assert_eq!(jr.case_id, "job:j1");
        assert_eq!(jr.actor, "system");

        // list filters by case; limit keeps the most recent.
        let all = svc.list(root, None, None).unwrap();
        assert_eq!(all.len(), 9);
        let case = svc.list(root, Some("jira:FIXY-1"), None).unwrap();
        assert_eq!(case.len(), 7);
        let last2 = svc.list(root, None, Some(2)).unwrap();
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[1].seq, 8);

        // Chain verifies end to end; seq/prev_hash link up.
        let v = svc.verify(root).unwrap();
        assert!(v.ok, "fresh chain must verify");
        assert_eq!(v.total, 9);
        assert_eq!(all[3].prev_hash, all[2].hash);

        // case_link bindings survive a "restart" (fresh service re-reads them).
        let svc2 = TestigoService::new();
        let p2 = svc2
            .on_prompt(root, 10, Some("t1"), None, Some("again"), None, None)
            .unwrap();
        assert_eq!(p2.case_id, "jira:FIXY-1", "binding rebuilt from ledger");
        assert_eq!(p2.seq, 9);
        assert!(svc2.verify(root).unwrap().ok, "cross-restart chain intact");

        // head() reports this instance's tail (feeds the Testigo-Head trailer).
        let (hseq, hhash) = svc2.head(root).unwrap().expect("non-empty ledger");
        assert_eq!(hseq, 9);
        assert_eq!(hhash, p2.hash);

        // Tampering with a middle line breaks verification at that seq.
        let path = TestigoService::ledger_path(root).unwrap();
        let tampered = fs::read_to_string(&path)
            .unwrap()
            .replace("looks safe", "totally legit");
        fs::write(&path, tampered).unwrap();
        let v = svc.verify(root).unwrap();
        assert!(!v.ok);
        assert_eq!(v.broken_at_seq, Some(3));

        // A crash-torn tail is tolerated by list AND reported by verify.
        fs::write(&path, "").unwrap();
        let svc3 = TestigoService::new();
        svc3.on_prompt(root, 9, Some("t3"), None, Some("x"), None, None)
            .unwrap();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{ half a line").unwrap();
        drop(f);
        assert_eq!(svc3.list(root, None, None).unwrap().len(), 1);
        let v = svc3.verify(root).unwrap();
        assert!(v.ok);
        assert!(v.torn_tail);

        // A fresh service self-heals the torn tail on first touch, so the next
        // append continues a linear, verifiable chain. Oversized tool inputs
        // are stored as a bounded preview.
        let big = "y".repeat(MAX_INPUT_BYTES * 2);
        let req = serde_json::json!({ "id": "ap2", "ts": 10, "tool": "bash", "input": { "cmd": big }, "termId": "t3" });
        let svc4 = TestigoService::new();
        let r = svc4.on_approval_request(root, &req).unwrap();
        assert_eq!(r.payload["input"]["truncated"], true);
        assert_eq!(r.seq, 1, "torn tail healed, chain continues from seq 0");
        let v = svc4.verify(root).unwrap();
        assert!(v.ok && !v.torn_tail, "healed chain verifies clean");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Per-project policy: defaults (witness on, repo marks OFF), persisted
    /// round-trip across service instances, and the witness switch actually
    /// blocking ledger writes while leaving reads of past evidence intact.
    #[test]
    fn settings_default_persist_and_gate() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-tsettings-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = TestigoService::new();
        let root = "/proj/shared";

        // Defaults: local witnessing on, repo marks off (shared-repo safe).
        let s = svc.settings(root);
        assert!(s.witness && !s.repo_marks);
        assert!(!svc.repo_marks(root));

        // Witnessing works by default…
        svc.on_prompt(root, 1, Some("t1"), None, Some("hi"), None, None)
            .unwrap();
        assert_eq!(svc.list(root, None, None).unwrap().len(), 1);

        // …and stops when switched off; past evidence stays readable.
        svc.set_settings(root, TestigoSettings { witness: false, ..Default::default() })
            .unwrap();
        assert!(svc
            .on_prompt(root, 2, Some("t1"), None, Some("nope"), None, None)
            .is_err());
        assert_eq!(svc.list(root, None, None).unwrap().len(), 1, "no new events");
        assert!(svc.verify(root).unwrap().ok, "reads unaffected");

        // Round-trip: a fresh instance loads the persisted policy.
        let svc2 = TestigoService::new();
        let s2 = svc2.settings(root);
        assert!(!s2.witness);
        // Enabling repo marks persists too.
        svc2.set_settings(root, TestigoSettings { witness: true, repo_marks: true, ..Default::default() })
            .unwrap();
        assert!(TestigoService::new().repo_marks(root));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// anchor_head pins {seq, hash, ts} at refs/agent-console/testigo-head in
    /// a real git repo, updates on re-anchor, and no-ops outside git. Separate
    /// test fn: it doesn't touch XDG_DATA_HOME, only a scratch git repo.
    #[test]
    fn anchor_head_pins_and_updates_ref() {
        use crate::services::proc;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo =
            std::env::temp_dir().join(format!("ac-anchor-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&repo).unwrap();
        assert!(proc::command("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .status
            .success());

        anchor_head(&repo, 7, "abc123", 111).unwrap();
        let show = proc::command("git")
            .args(["cat-file", "-p", "refs/agent-console/testigo-head"])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(show.status.success(), "ref must exist and point at the blob");
        let body = String::from_utf8_lossy(&show.stdout);
        assert!(body.contains("\"seq\":7") && body.contains("abc123"), "{body}");

        // Re-anchor moves the ref to the new head.
        anchor_head(&repo, 8, "def456", 222).unwrap();
        let show = proc::command("git")
            .args(["cat-file", "-p", "refs/agent-console/testigo-head"])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&show.stdout).contains("def456"));

        // Non-git dir: best-effort no-op, never an error.
        let plain =
            std::env::temp_dir().join(format!("ac-anchor-plain-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&plain).unwrap();
        anchor_head(&plain, 1, "x", 1).unwrap();

        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_dir_all(&plain);
    }
}
