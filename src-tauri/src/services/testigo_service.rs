use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};

use parking_lot::Mutex;
use regex::Regex;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

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

/// `"hash":"<64 lowercase hex>"}` as the FINAL member of the line, tolerating
/// whitespace around it (JSON allows it; the spec says "preserve every byte").
static FINAL_HASH_MEMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"("hash"\s*:\s*")([0-9a-f]{64})("\s*\}\s*)$"#).expect("static regex")
});

/// Byte-exact recompute of a raw ledger line's content hash (spec §1.5).
///
/// `hash` MUST be the final member and its value 64 lowercase hex characters;
/// only that value is emptied, every other byte of the line is hashed as-is.
/// Never rebuild the suffix from the last `"hash":"` occurrence: a member
/// appended after `hash` (a second `payload`, say) would vanish from the
/// recomputation while changing what the line parses to — the hash and the
/// linkage would still "verify". Returns `None` when the line does not end in
/// a well-formed final `hash` member, which is itself a verification failure.
pub fn recompute_line_hash(line: &str) -> Option<String> {
    let m = FINAL_HASH_MEMBER.captures(line)?;
    let start = m.get(0)?.start();
    let unhashed = format!("{}{}{}", &line[..start], &m[1], &m[3]);
    let mut h = Sha256::new();
    h.update(unhashed.as_bytes());
    Some(format!("{:x}", h.finalize()))
}

/// Instruction files an agent reads implicitly (spec §1.7, `prompt.payload.context`).
pub const INSTRUCTION_FILES: &[&str] = &["CLAUDE.md", ".claude/CLAUDE.md", "AGENTS.md"];

/// `[{uri, sha256}]` for the instruction files present under `cwd` at call
/// time — the bytes the agent actually ran under, not whatever is on disk
/// later. Absent or unreadable files are simply not instructions it had.
pub fn instruction_context(cwd: &str) -> Vec<Value> {
    INSTRUCTION_FILES
        .iter()
        .filter_map(|rel| {
            let bytes = fs::read(Path::new(cwd).join(rel)).ok()?;
            let mut h = Sha256::new();
            h.update(&bytes);
            Some(json!({ "uri": rel, "sha256": format!("{:x}", h.finalize()) }))
        })
        .collect()
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

/// Correlation ids and run facts a PostToolUse / PostToolUseFailure payload
/// may carry (all optional: older CLIs and Codex send few of them).
#[derive(Debug, Default, Clone, Copy)]
pub struct ToolResultIds<'a> {
    pub tool_use_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    /// The Bash command line (what makes a result a check run).
    pub command: Option<&'a str>,
    /// sha256 of the FULL output the excerpt was cut from.
    pub output_sha256: Option<&'a str>,
    /// PostToolUseFailure: the tool ran and failed.
    pub failed: bool,
    pub exit_code: Option<i64>,
    pub interrupted: Option<bool>,
    pub duration_ms: Option<u64>,
}

/// Does this Bash command line run a test/check suite? Conservative
/// allow-list of common runners; a hit turns the result into a `check_run`
/// event — the "tests ran, and this is what they said" line a reviewer
/// reads before any prompt.
pub fn is_check_command(cmd: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"(?x)(^|[\s;&|(])(
                cargo\s+(test|clippy|check|fmt\s+--check)
              | (npm|pnpm|yarn|bun)\s+(test|run\s+(test|tests|lint|typecheck|check|build|format:check))
              | npx\s+(vitest|jest|playwright|tsc|eslint|prettier\s+--check|mocha)
              | (vitest|jest|mocha|playwright\s+test|pytest|tox|nox|rspec|phpunit|dotnet\s+test|swift\s+test)
              | python(3)?\s+-m\s+(pytest|unittest)
              | go\s+(test|vet)
              | make\s+(test|check|lint)
              | (mvn|mvnw|\./mvnw)\s+(test|verify)
              | (gradle|gradlew|\./gradlew)\s+(test|check)
              | mix\s+test
              | bundle\s+exec\s+rspec
            )(\s|$)",
        )
        .expect("check-runner regex compiles")
    });
    re.is_match(cmd)
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
    /// approval id -> (term_id, tool, project_root), so the decision event
    /// can name what it approved — and land in the SAME ledger the request
    /// did — even though the respond() path only carries the id.
    approvals: HashMap<String, (Option<String>, Option<String>, String)>,
    /// term_id -> corpus doc ids the inject endpoint just handed the agent.
    /// Consumed by the NEXT prompt of that terminal: the injection happens
    /// inside the UserPromptSubmit hook, before the prompt event reaches the
    /// watcher, so binding it to "the open turn" would pin it to the previous
    /// one. Parking it until the prompt opens its turn gets the attribution
    /// right by construction.
    pending_injections: HashMap<String, Vec<String>>,
    /// (session_id, model) pairs already recorded as `session_start`, so the
    /// status line's per-render model report yields one line per session,
    /// not one per render.
    seen_session_models: std::collections::HashSet<(String, String)>,
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
    pub fn set_settings(
        &self,
        project_root: &str,
        s: TestigoSettings,
    ) -> AppResult<TestigoSettings> {
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
            return Err(AppError::Other(
                "testigo: witnessing disabled for this project".into(),
            ));
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
        // Spec §1.7 (v0.2): the instruction files present in the prompt's
        // cwd, hashed NOW — inside the chain — so the packet's context
        // artifacts are derived from evidence, not typed in at export.
        let mut payload = json!({ "prompt": prompt, "skill": skill, "cwd": cwd });
        if let Some(dir) = cwd {
            let ctx = instruction_context(dir);
            if !ctx.is_empty() {
                payload["context"] = Value::Array(ctx);
            }
        }
        let ev = Self::record(
            &mut inner,
            project_root,
            ts,
            case.clone(),
            Some(turn_id.clone()),
            "prompt",
            term_id.map(String::from),
            session_id.map(String::from),
            "human",
            payload,
        )?;
        // What the inject endpoint fed this very prompt (memories/skills), now
        // that the turn exists to hang it on. Evidence of influence: the
        // packet can say which memory shaped this turn — the hook the flywheel
        // needs to measure itself (F1).
        let docs = term_id.and_then(|t| inner.pending_injections.remove(t));
        if let Some(docs) = docs.filter(|d| !d.is_empty()) {
            let _ = Self::record(
                &mut inner,
                project_root,
                ts,
                case,
                Some(turn_id),
                "context_injected",
                term_id.map(String::from),
                session_id.map(String::from),
                "system",
                json!({ "docs": docs }),
            );
        }
        Ok(ev)
    }

    /// The inject endpoint handed `doc_ids` to the agent for `term_id`'s
    /// next prompt. Parked, not recorded: see `pending_injections`.
    pub fn note_injection(&self, term_id: &str, doc_ids: Vec<String>) {
        self.inner
            .lock()
            .pending_injections
            .insert(term_id.to_string(), doc_ids);
    }

    /// PostModelSwitch: the session changed model mid-conversation. The
    /// export reads `payload.to` into the packet's `languageModels`.
    #[allow(clippy::too_many_arguments)]
    pub fn on_model_switch(
        &self,
        project_root: &str,
        ts: i64,
        term_id: Option<&str>,
        session_id: Option<&str>,
        to: &str,
        from: Option<&str>,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        let turn_id = term_id
            .and_then(|t| inner.turns.get(t))
            .map(|s| s.turn_id.clone());
        let case = Self::case_for(&inner, term_id);
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "model_switch",
            term_id.map(String::from),
            session_id.map(String::from),
            "agent",
            json!({ "to": to, "from": from }),
        )
    }

    /// The model a session runs, as the CLI's own status line reports it
    /// (T3). Recorded once per (session, model) as `session_start` — the kind
    /// the export reads `payload.model` from — so packets carry
    /// `languageModels` even when no switch ever happened. Returns None when
    /// this pair was already recorded.
    pub fn on_session_model(
        &self,
        project_root: &str,
        ts: i64,
        term_id: Option<&str>,
        session_id: &str,
        model: &str,
    ) -> AppResult<Option<ProofEvent>> {
        let mut inner = self.inner.lock();
        let key = (session_id.to_string(), model.to_string());
        if inner.seen_session_models.contains(&key) {
            return Ok(None);
        }
        let turn_id = term_id
            .and_then(|t| inner.turns.get(t))
            .map(|s| s.turn_id.clone());
        let case = Self::case_for(&inner, term_id);
        let ev = Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "session_start",
            term_id.map(String::from),
            Some(session_id.to_string()),
            "system",
            json!({ "model": model, "observedVia": "statusline" }),
        )?;
        inner.seen_session_models.insert(key);
        Ok(Some(ev))
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
        let turn_id = term_id
            .and_then(|t| inner.turns.remove(t))
            .map(|s| s.turn_id);
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
        ids: ToolResultIds<'_>,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        let turn_id = term_id
            .and_then(|t| inner.turns.get(t))
            .map(|s| s.turn_id.clone());
        let case = Self::case_for(&inner, term_id);
        let excerpt_v = bounded_input(json!(excerpt));
        let mut payload = json!({ "tool": tool, "excerpt": excerpt_v, "truncated": truncated });
        // Correlation handles the hook payload carries (Claude 2.1.x): the
        // tool_use_id pairs this result with its request; agent_id says it
        // ran inside a subagent (a Task), not the session's main thread —
        // until now those 30-odd calls per case read as the parent's own.
        if let Some(id) = ids.tool_use_id.filter(|s| !s.is_empty()) {
            payload["toolUseId"] = json!(id);
        }
        if let Some(id) = ids.agent_id.filter(|s| !s.is_empty()) {
            payload["agentId"] = json!(id);
        }
        if let Some(c) = ids.command.filter(|s| !s.is_empty()) {
            payload["command"] = json!(c);
        }
        if let Some(h) = ids.output_sha256.filter(|s| !s.is_empty()) {
            payload["outputSha256"] = json!(h);
        }
        if ids.failed {
            payload["failed"] = json!(true);
        }
        if let Some(code) = ids.exit_code {
            payload["exitCode"] = json!(code);
        }
        if let Some(i) = ids.interrupted {
            payload["interrupted"] = json!(i);
        }
        if let Some(d) = ids.duration_ms {
            payload["durationMs"] = json!(d);
        }
        let ev = Self::record(
            &mut inner,
            project_root,
            ts,
            case.clone(),
            turn_id.clone(),
            "tool_result",
            term_id.map(String::from),
            session_id.map(String::from),
            "agent",
            payload,
        )?;
        // A recognized test/check runner also leaves a `check_run` line:
        // command, verdict, exit code and the hash of its full output.
        if tool == Some("Bash") {
            if let Some(cmd) = ids.command.filter(|c| is_check_command(c)) {
                let status = if ids.interrupted == Some(true) {
                    "interrupted"
                } else if ids.failed {
                    "failed"
                } else {
                    "passed"
                };
                let _ = Self::record(
                    &mut inner,
                    project_root,
                    ts,
                    case,
                    turn_id,
                    "check_run",
                    term_id.map(String::from),
                    session_id.map(String::from),
                    "agent",
                    json!({
                        "command": cmd,
                        "status": status,
                        "exitCode": ids.exit_code,
                        "outputSha256": ids.output_sha256,
                        "toolUseId": ids.tool_use_id,
                        "durationMs": ids.duration_ms,
                    }),
                );
            }
        }
        Ok(ev)
    }

    /// The human committed: the ledger's own record of "this work reached
    /// git", bound to the case and turn whose diff produced the staged files
    /// (same rule as the Testigo-Case trailer, but recorded whether or not
    /// trailers are on). `None` turn ⇒ no recorded turn touched these files
    /// within the window; the commit still records under "unbound".
    #[allow(clippy::too_many_arguments)]
    pub fn on_commit(
        &self,
        project_root: &str,
        ts: i64,
        sha: &str,
        subject: &str,
        files: &[String],
        amend: bool,
        max_age_ms: i64,
    ) -> AppResult<ProofEvent> {
        let binding = self.turn_for_files(project_root, files, ts, max_age_ms)?;
        let (case, turn_id) = match binding {
            Some((c, t)) => (c, Some(t)),
            None => ("unbound".to_string(), None),
        };
        let mut inner = self.inner.lock();
        Self::ensure_tail(&mut inner, project_root)?;
        let files_v: Vec<Value> = files.iter().take(500).map(|f| json!(f)).collect();
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "commit",
            None,
            None,
            "human",
            json!({
                "sha": sha,
                "subject": subject.lines().next().unwrap_or("").chars().take(200).collect::<String>(),
                "files": files_v,
                "filesTruncated": files.len() > 500,
                "amend": amend,
            }),
        )
    }

    /// Like `case_for_files`, but also names the turn: the most recent
    /// `turn_end` within `max_age_ms` whose diff touched any of `files`.
    pub fn turn_for_files(
        &self,
        project_root: &str,
        files: &[String],
        now_ms: i64,
        max_age_ms: i64,
    ) -> AppResult<Option<(String, String)>> {
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
                if let Some(t) = &ev.turn_id {
                    return Ok(Some((ev.case_id.clone(), t.clone())));
                }
            }
        }
        Ok(None)
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

    /// Record a turn rewind: the working tree was restored to a snapshot and
    /// (when the fork succeeded) the conversation transcript was forked to a
    /// new session id. A rewind rewrites what the checkout contains, so it is
    /// an auditable fact in its own right — the payload names the restored
    /// snapshot, the original session and the fork, and `turn_id` points at
    /// the ledger turn the user rewound to.
    pub fn on_rewind(
        &self,
        project_root: &str,
        ts: i64,
        term_id: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<String>,
        payload: Value,
    ) -> AppResult<ProofEvent> {
        let mut inner = self.inner.lock();
        Self::ensure_tail(&mut inner, project_root)?;
        let case = Self::case_for(&inner, term_id);
        Self::record(
            &mut inner,
            project_root,
            ts,
            case,
            turn_id,
            "rewind",
            term_id.map(String::from),
            session_id.map(String::from),
            "human",
            payload,
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
        inner.approvals.insert(
            id.to_string(),
            (term_id.clone(), tool.clone(), project_root.to_string()),
        );
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
        // (empty map) still records, just without tool/term context — and in
        // the caller's project, since the request's is unknown by then.
        let (term_id, tool, root) =
            inner
                .approvals
                .remove(id)
                .unwrap_or((None, None, project_root.to_string()));
        let project_root: &str = &root;
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

    /// Snapshot commits that must stay restorable no matter how old the pin is:
    /// the pre/post pair of each terminal's most recent closed turn, plus the
    /// pre-snapshot of any turn still open. That is the undo a user can still
    /// reach for — "undo the last thing this session did" — and retention
    /// (`snapshot_service::sweep`) must not be the thing that takes it away.
    ///
    /// Everything else the ledger names is evidence, not a live handle: the shas
    /// and the file list are already recorded, and export reads ledger lines
    /// rather than git objects, so an expired pin costs no proof.
    ///
    /// Best-effort by design — an unreadable ledger yields an empty set, which
    /// only ever makes the sweep *more* conservative at its other guards.
    pub fn live_snapshot_shas(&self, project_root: &str) -> HashSet<String> {
        // Scoped: `list` takes the same (non-reentrant) lock.
        let mut keep: HashSet<String> = {
            let inner = self.inner.lock();
            inner
                .turns
                .values()
                .filter_map(|t| t.pre_sha.clone())
                .collect()
        };

        let Ok(events) = self.list(project_root, None, None) else {
            return keep;
        };
        // Last turn_end wins per terminal: the list is chronological, so a later
        // close simply overwrites the pair recorded for that term.
        let mut last_by_term: HashMap<String, Vec<String>> = HashMap::new();
        for ev in events.iter().filter(|e| e.kind == "turn_end") {
            let term = ev.term_id.clone().unwrap_or_else(|| "unbound".into());
            let shas = ["preSha", "postSha"]
                .iter()
                .filter_map(|k| ev.payload.get(*k).and_then(|v| v.as_str()))
                .map(String::from)
                .collect();
            last_by_term.insert(term, shas);
        }
        keep.extend(last_by_term.into_values().flatten());
        keep
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
            // Recompute over the RAW bytes (spec §1.5). Round-tripping through
            // the struct would silently drop a member appended after `hash`.
            let recomputed = recompute_line_hash(line);
            if ev.prev_hash != expected_prev
                || ev.seq != expected_seq
                || recomputed.as_deref() != Some(ev.hash.as_str())
            {
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

    /// T4a: the pieces that make a packet say WHICH model ran, WHICH memory
    /// shaped a turn, which result belongs to which request — and that a
    /// decision lands in the ledger its request came from, whatever project
    /// the UI shows when the human clicks.
    #[test]
    fn model_events_injections_ids_and_decision_attribution() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-testigo-t4a-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = TestigoService::new();
        let a = "/proj/a";
        let b = "/proj/b";

        // An injection parked before the prompt lands right after it, inside
        // ITS turn — not the previous one.
        svc.note_injection("t1", vec!["memory:release.md".into()]);
        let p = svc
            .on_prompt(a, 1, Some("t1"), Some("s1"), Some("go"), None, None)
            .unwrap();
        let events = svc.list(a, None, None).unwrap();
        let ci = events
            .iter()
            .find(|e| e.kind == "context_injected")
            .expect("context_injected recorded");
        assert_eq!(ci.turn_id, p.turn_id);
        assert_eq!(ci.payload["docs"][0], "memory:release.md");
        assert_eq!(ci.actor, "system");
        assert_eq!(ci.seq, p.seq + 1, "immediately after the prompt");
        // A prompt with nothing parked records no injection line.
        svc.on_prompt(a, 2, Some("t1"), Some("s1"), Some("more"), None, None)
            .unwrap();
        let n = svc
            .list(a, None, None)
            .unwrap()
            .iter()
            .filter(|e| e.kind == "context_injected")
            .count();
        assert_eq!(n, 1);

        // session_start once per (session, model); a new model is a new line.
        assert!(svc
            .on_session_model(a, 3, Some("t1"), "s1", "claude-opus-5")
            .unwrap()
            .is_some());
        assert!(svc
            .on_session_model(a, 4, Some("t1"), "s1", "claude-opus-5")
            .unwrap()
            .is_none());
        let hk = svc
            .on_session_model(a, 5, Some("t1"), "s1", "claude-haiku-4-5")
            .unwrap()
            .unwrap();
        assert_eq!(hk.kind, "session_start");
        assert_eq!(hk.payload["model"], "claude-haiku-4-5");
        assert_eq!(hk.session_id.as_deref(), Some("s1"));
        // model_switch carries `to` (what the export reads) and `from`.
        let ms = svc
            .on_model_switch(
                a,
                6,
                Some("t1"),
                Some("s1"),
                "claude-sonnet-5",
                Some("claude-opus-5"),
            )
            .unwrap();
        assert_eq!(ms.kind, "model_switch");
        assert_eq!(ms.payload["to"], "claude-sonnet-5");
        assert_eq!(ms.payload["from"], "claude-opus-5");
        assert!(ms.turn_id.is_some(), "inside the open turn");

        // tool_result carries the correlation ids when the hook had them…
        let tr = svc
            .on_tool_result(
                a,
                7,
                Some("t1"),
                Some("s1"),
                Some("Bash"),
                Some("ok"),
                false,
                ToolResultIds {
                    tool_use_id: Some("toolu_9"),
                    agent_id: Some("sub-1"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(tr.payload["toolUseId"], "toolu_9");
        assert_eq!(tr.payload["agentId"], "sub-1");
        // …and no keys at all when it didn't (older CLI, Codex).
        let bare = svc
            .on_tool_result(
                a,
                8,
                Some("t1"),
                Some("s1"),
                Some("Read"),
                Some("x"),
                false,
                ToolResultIds::default(),
            )
            .unwrap();
        assert!(bare.payload.get("toolUseId").is_none());
        assert!(bare.payload.get("agentId").is_none());

        // A request filed under project b is decided while the UI shows a:
        // the decision goes to b's ledger, where the request is.
        let req = json!({
            "id": "apX", "ts": 9, "tool": "Bash", "input": {}, "cwd": "/proj/b", "termId": "t2"
        });
        svc.on_approval_request(b, &req).unwrap();
        let d = svc
            .on_approval_decision(a, 10, "apX", "deny", None)
            .unwrap();
        assert_eq!(d.payload["tool"], "Bash");
        let in_b = svc
            .list(b, None, None)
            .unwrap()
            .iter()
            .any(|e| e.kind == "approval_decision");
        let in_a = svc
            .list(a, None, None)
            .unwrap()
            .iter()
            .any(|e| e.kind == "approval_decision");
        assert!(in_b && !in_a);
        // A recognized runner leaves a check_run beside its tool_result:
        // passed on PostToolUse, failed (with the exit code) on failure.
        let ok = svc
            .on_tool_result(
                a,
                12,
                Some("t1"),
                Some("s1"),
                Some("Bash"),
                Some("test result: ok"),
                false,
                ToolResultIds {
                    command: Some("cargo test --workspace"),
                    output_sha256: Some("ab".repeat(32).as_str()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(ok.payload["command"], "cargo test --workspace");
        let evs = svc.list(a, None, None).unwrap();
        let cr = evs.iter().rev().find(|e| e.kind == "check_run").unwrap();
        assert_eq!(cr.payload["status"], "passed");
        assert_eq!(cr.payload["command"], "cargo test --workspace");
        assert_eq!(cr.turn_id, ok.turn_id);
        assert_eq!(cr.seq, ok.seq + 1);
        let failed = svc
            .on_tool_result(
                a,
                13,
                Some("t1"),
                Some("s1"),
                Some("Bash"),
                Some("Exit code 1\nFAIL"),
                false,
                ToolResultIds {
                    command: Some("npm test"),
                    failed: true,
                    exit_code: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(failed.payload["failed"], true);
        assert_eq!(failed.payload["exitCode"], 1);
        let evs = svc.list(a, None, None).unwrap();
        let cr = evs.iter().rev().find(|e| e.kind == "check_run").unwrap();
        assert_eq!(cr.payload["status"], "failed");
        assert_eq!(cr.payload["exitCode"], 1);
        // A plain command is not a check.
        svc.on_tool_result(
            a,
            14,
            Some("t1"),
            Some("s1"),
            Some("Bash"),
            Some("x"),
            false,
            ToolResultIds {
                command: Some("ls -la"),
                ..Default::default()
            },
        )
        .unwrap();
        let checks = svc
            .list(a, None, None)
            .unwrap()
            .iter()
            .filter(|e| e.kind == "check_run")
            .count();
        assert_eq!(checks, 2);

        // A commit binds to the turn whose diff touched the staged files.
        let turn_a = svc
            .on_prompt(a, 15, Some("t1"), Some("s1"), Some("edit"), None, None)
            .unwrap()
            .turn_id
            .unwrap();
        svc.on_turn_end(
            a,
            16,
            Some("t1"),
            Some("s1"),
            json!({ "filesChanged": [{ "status": "M", "path": "src/lib.rs" }] }),
        )
        .unwrap();
        let c = svc
            .on_commit(
                a,
                17,
                "deadbeef",
                "Fix the thing\n\nBody",
                &["src/lib.rs".to_string()],
                false,
                60_000,
            )
            .unwrap();
        assert_eq!(c.kind, "commit");
        assert_eq!(c.actor, "human");
        assert_eq!(c.turn_id.as_deref(), Some(turn_a.as_str()));
        assert_eq!(c.payload["sha"], "deadbeef");
        assert_eq!(c.payload["subject"], "Fix the thing");
        assert_eq!(c.payload["files"][0], "src/lib.rs");
        assert_eq!(c.payload["amend"], false);
        // Files no turn touched ⇒ recorded, but unbound.
        let orphan_commit = svc
            .on_commit(
                a,
                18,
                "cafe",
                "docs",
                &["README.md".to_string()],
                true,
                60_000,
            )
            .unwrap();
        assert_eq!(orphan_commit.case_id, "unbound");
        assert!(orphan_commit.turn_id.is_none());
        assert_eq!(orphan_commit.payload["amend"], true);

        // An unknown id (post-restart) still records — in the caller's ledger.
        let orphan = svc
            .on_approval_decision(a, 11, "never-seen", "allow", None)
            .unwrap();
        assert!(orphan.payload["tool"].is_null());
        assert!(svc.verify(a).unwrap().ok);
        assert!(svc.verify(b).unwrap().ok);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn check_command_allowlist_matches_runners_not_plain_shell() {
        for c in [
            "cargo test",
            "cargo test --workspace -- --nocapture",
            "cd src-tauri && cargo clippy -- -D warnings",
            "npm test",
            "npm run lint",
            "pnpm run typecheck",
            "npx vitest run",
            "pytest -q tests/",
            "python -m pytest",
            "go test ./...",
            "make check",
            "./gradlew test",
            "bundle exec rspec",
        ] {
            assert!(is_check_command(c), "{c}");
        }
        for c in [
            "ls -la",
            "git status",
            "npm install",
            "cargo build --release",
            "echo test",
            "cat pytest.ini",
            "mytest",
        ] {
            assert!(!is_check_command(c), "{c}");
        }
    }

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
        let s = svc
            .on_snapshot(root, 5, Some("t1"), Some("s1"), "abc123")
            .unwrap();
        assert_eq!(s.turn_id.as_deref(), Some(turn.as_str()));
        let peeked = svc.peek_turn("t1").unwrap();
        assert_eq!(peeked.pre_sha.as_deref(), Some("abc123"));

        // A tool result lands inside the same turn, excerpt preserved.
        let tr = svc
            .on_tool_result(
                root,
                6,
                Some("t1"),
                Some("s1"),
                Some("Bash"),
                Some("ok\n"),
                false,
                ToolResultIds {
                    tool_use_id: Some("toolu_1"),
                    agent_id: None,
                    ..Default::default()
                },
            )
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
        let jr = svc
            .on_job_run(root, 9, "j1", "nightly", "ok", "all good")
            .unwrap();
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

        // Retention keep-set: what snapshot pins must outlive their expiry.
        // Right now t1's newest CLOSED turn is the one from seq 7, and the turn
        // p2 just opened has no snapshot yet.
        let keep = svc2.live_snapshot_shas(root);
        assert!(
            keep.contains("abc123") && keep.contains("def456"),
            "the last closed turn's pre/post pair is the live undo"
        );

        // A snapshot taken inside the OPEN turn is kept too — it's the "before"
        // of an undo the user can still reach for, and it's in no closed turn.
        svc2.on_snapshot(root, 11, Some("t1"), None, "open789")
            .unwrap();
        assert!(svc2.live_snapshot_shas(root).contains("open789"));

        // Closing that turn moves the keep-set forward: the previous pair is now
        // ordinary history — evidence in the ledger, but no longer a live handle,
        // so retention is free to expire it.
        svc2.on_turn_end(
            root,
            12,
            Some("t1"),
            None,
            serde_json::json!({ "preSha": "open789", "postSha": "post789" }),
        )
        .unwrap();
        let keep = svc2.live_snapshot_shas(root);
        assert!(keep.contains("open789") && keep.contains("post789"));
        assert!(
            !keep.contains("abc123") && !keep.contains("def456"),
            "only the MOST RECENT turn per terminal is pinned past expiry"
        );

        // A project with no ledger keeps nothing — and says so instead of failing.
        assert!(TestigoService::new()
            .live_snapshot_shas("/proj/empty")
            .is_empty());

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

    /// Spec §1.5 (testigo#7): a member appended AFTER `hash` must break
    /// verification. The struct round-trip used to drop it (serde ignores
    /// unknown fields), so the stored hash still "recomputed" while the
    /// parsed event had changed. `hash` must be the final member and every
    /// other byte of the line is covered.
    #[test]
    fn verify_rejects_members_appended_after_hash() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "ac-testigo-suffix-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = TestigoService::new();
        let root = "/proj/suffix";
        svc.on_prompt(root, 1, Some("t1"), None, Some("first"), None, None)
            .unwrap();
        svc.on_prompt(root, 2, Some("t1"), None, Some("second"), None, None)
            .unwrap();
        let path = TestigoService::ledger_path(root).unwrap();
        let original = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = original.lines().collect();
        assert_eq!(lines.len(), 2);

        // The sealed lines recompute byte-exactly; malformed final members don't.
        for line in &lines {
            let v: Value = serde_json::from_str(line).unwrap();
            assert_eq!(
                recompute_line_hash(line).as_deref(),
                Some(v["hash"].as_str().unwrap())
            );
        }
        let no_final_hash = format!("{},\"x\":1}}", &lines[0][..lines[0].len() - 1]);
        assert_eq!(recompute_line_hash(&no_final_hash), None);
        assert_eq!(recompute_line_hash(&lines[0].to_uppercase()), None);
        assert_eq!(recompute_line_hash("{}"), None);

        // Each mutation keeps the stored hash and the linkage intact — only a
        // byte-exact recompute catches it. Index 0 is a middle line; index 1
        // is the tail, where an unparseable line would be tolerated as torn —
        // but an appended member still PARSES, so it must be caught there too.
        let mutations: [(&str, fn(&str) -> String); 4] = [
            ("member after hash", |l| {
                format!("{},\"unhashed\":true}}", &l[..l.len() - 1])
            }),
            ("duplicate payload", |l| {
                format!(
                    "{},\"payload\":{{\"prompt\":\"replaced\"}}}}",
                    &l[..l.len() - 1]
                )
            }),
            ("escaped duplicate payload", |l| {
                format!(
                    "{},\"paylo\\u0061d\":{{\"prompt\":\"replaced\"}}}}",
                    &l[..l.len() - 1]
                )
            }),
            ("trailing whitespace", |l| format!("{} \t", l)),
        ];
        for (name, mutate) in mutations {
            for idx in 0..2 {
                let mut tampered: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
                tampered[idx] = mutate(&tampered[idx]);
                fs::write(&path, tampered.join("\n") + "\n").unwrap();
                let v = TestigoService::new().verify(root).unwrap();
                if v.ok {
                    // serde rejects a duplicate key outright; on the FINAL
                    // line that is tolerated as a torn tail — the event is
                    // dropped from the chain, never accepted with new content.
                    assert!(
                        idx == 1 && v.torn_tail && v.total == 1,
                        "{name} at index {idx}: only a torn tail may be tolerated"
                    );
                } else {
                    assert!(
                        !v.torn_tail,
                        "{name} at index {idx} is tampering, not a torn tail"
                    );
                    assert_eq!(v.broken_at_seq, Some(idx as u64), "{name} at index {idx}");
                }
            }
        }

        // Untouched, the chain still verifies clean.
        fs::write(&path, &original).unwrap();
        let v = TestigoService::new().verify(root).unwrap();
        assert!(v.ok && !v.torn_tail && v.total == 2);

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
        svc.set_settings(
            root,
            TestigoSettings {
                witness: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(svc
            .on_prompt(root, 2, Some("t1"), None, Some("nope"), None, None)
            .is_err());
        assert_eq!(
            svc.list(root, None, None).unwrap().len(),
            1,
            "no new events"
        );
        assert!(svc.verify(root).unwrap().ok, "reads unaffected");

        // Round-trip: a fresh instance loads the persisted policy.
        let svc2 = TestigoService::new();
        let s2 = svc2.settings(root);
        assert!(!s2.witness);
        // Enabling repo marks persists too.
        svc2.set_settings(
            root,
            TestigoSettings {
                witness: true,
                repo_marks: true,
                ..Default::default()
            },
        )
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
        assert!(
            show.status.success(),
            "ref must exist and point at the blob"
        );
        let body = String::from_utf8_lossy(&show.stdout);
        assert!(
            body.contains("\"seq\":7") && body.contains("abc123"),
            "{body}"
        );

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
