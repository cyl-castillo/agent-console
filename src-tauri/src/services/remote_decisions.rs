//! F8 (COMPETENCIA-2026-10): decisions on a paired device.
//!
//! The remote client shows DecisionCards with their evidence and returns a
//! decision — nothing else: no chat, no diffs, no prompts. This module is
//! the application layer over the Noise channel (`transport.rs`): what a
//! pending approval looks like on the wire (`RemoteDecision`), the tier the
//! desktop computes for it (conservative, mirrors `lib/decisions.ts`), the
//! gate that says what a device of a given scope may answer, and the bridge
//! into the console's own approval queue and ledger.
//!
//! The gate is the point: a dangerous (high-tier) action is **never**
//! approved from afar — `deny` is always possible, `allow` needs the
//! keyboard. Per-device scope narrows further (`ReadOnly` may only look).

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::paired_devices_service::{DeviceScope, PairedDevice, PairedDevicesService};
use crate::services::testigo_service::ProofEvent;

/// Longest command / path carried to the device.
pub const SUMMARY_CAP: usize = 300;
/// Most decisions in one `Decisions` reply (newest first, `truncated` set
/// beyond it) — keeps the frame well under the transport's 64 KiB cap.
pub const PENDING_CAP: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LastCheck {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub status: String,
}

/// What the device sees next to the request: the console's verdict on the
/// session's last closed turn and its last witnessed check. Evidence, not
/// a summary — the same ledger lines the Inbox card reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<LastCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDecision {
    pub id: String,
    pub ts: i64,
    pub tool: String,
    /// The command, the path, the URL — bounded.
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub project: String,
    /// low · medium · high
    pub tier: String,
    pub tier_label: String,
    pub tier_reason: String,
    /// False for a high tier: the device may deny, never allow.
    pub remote_allow: bool,
    pub evidence: Evidence,
}

static DANGEROUS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    [
        (
            r"(?i)curl[^|#\n]*\|\s*(ba)?sh",
            "downloads and executes remote code",
        ),
        (
            r"(?i)wget[^|#\n]*\|\s*(ba)?sh",
            "downloads and executes remote code",
        ),
        (
            r"(?i)>\s*/dev/(sda|hda|nvme|disk)\d*",
            "raw block device write",
        ),
        (r"(?i)\bdd\s+if=", "raw block device write"),
        (r"(?i)\bmkfs\b", "formats a filesystem"),
    ]
    .into_iter()
    .map(|(re, why)| (Regex::new(re).expect("static regex"), why))
    .collect()
});

static CAUTION: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    [
        (r"(?i)\brm\s+-[a-z]*[rf]", "recursive or force-delete"),
        (
            r"(?i)\bgit\s+push\b.*?(--force|-f)\b",
            "force-push rewrites remote history",
        ),
        (r"(?i)\bgit\s+push\b", "pushes to remote"),
        (
            r"(?i)\bgit\s+reset\s+--hard\b",
            "discards uncommitted changes",
        ),
        (r"(?i)\bgit\s+rebase\b", "rewrites commit history"),
        (r"(?i)\bgit\s+clean\s+-[a-z]*f", "removes untracked files"),
        (r"(?i)\bgit\s+restore\b", "discards working-tree changes"),
        (
            r"(?i)\bgit\s+checkout\s+(--\s|\.(\s|$))",
            "discards working-tree changes",
        ),
        (r"\bgit\s+branch\s+-[a-z]*D", "force-deletes a branch"),
        (r"(?i)\btruncate\b", "truncates a file"),
        (r"(?i)\bfind\b[^|]*\s-delete\b", "deletes matched files"),
        (r"(?i)\bsudo\b", "elevated privileges"),
        (r"(?i)\bchmod\b", "changes file permissions"),
        (
            r"(?i)\bapt(-get)?\s+(install|remove|purge)\b",
            "modifies system packages",
        ),
        (
            r"(?i)\bnpm\s+(install|uninstall)\s+-g\b",
            "modifies global packages",
        ),
        (r"(?i)\bpip\s+install\b", "installs Python package"),
        (r"(?i)\byarn\s+global\b", "modifies global packages"),
        (
            r"(?i)\bdocker\s+(rm|rmi|system\s+prune)\b",
            "destroys containers or images",
        ),
        (
            r"(?i)\bkubectl\s+(delete|drain)\b",
            "removes Kubernetes resources",
        ),
    ]
    .into_iter()
    .map(|(re, why)| (Regex::new(re).expect("static regex"), why))
    .collect()
});

static HIGH_REASON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)remote|delete|discards|destroys|removes|elevated|truncates|rewrites")
        .expect("static regex")
});

static READ_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(ls|pwd|cat|head|tail|less|wc|echo|which|type|env|printenv|date|whoami|stat|file|du|df|tree|rg|grep|egrep|fgrep|git\s+(status|log|diff|show|rev-parse|describe|blame|stash\s+list)|git\s+remote\s+-v|npm\s+(test|run\s+test|run\s+lint|run\s+build|ls|outdated|view)|npx\s+(tsc|vitest|eslint|prettier\s+--check|playwright\s+test)|cargo\s+(check|test|clippy|fmt\s+--check|tree|metadata)|node\s+-e|python3?\s+-c|make\s+test|pytest|go\s+(test|vet|build))\b",
    )
    .expect("static regex")
});

/// The two allowlist entries `lib/decisions.ts` writes with a negative
/// lookahead (`find` unless it deletes, `git branch` unless it deletes) —
/// the regex crate has none, so they are spelled out here to keep parity.
static FIND_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*find\b").expect("static"));
static FIND_DELETE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^|]*\s-delete\b").expect("static"));
static GIT_BRANCH_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*git\s+branch\b").expect("static"));
static GIT_BRANCH_DELETE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*git\s+branch\s+-[a-zA-Z]*[dD]").expect("static"));

fn read_only_bash(cmd: &str) -> bool {
    READ_ONLY.is_match(cmd)
        || (FIND_HEAD.is_match(cmd) && !FIND_DELETE.is_match(cmd))
        || (GIT_BRANCH_HEAD.is_match(cmd) && !GIT_BRANCH_DELETE.is_match(cmd))
}

/// The tier the desktop computes for a request before it leaves the machine.
/// Mirrors `lib/decisions.ts` for Bash (same patterns, same allowlist, the
/// two lookahead cases spelled out in `read_only_bash`); every other tool is
/// medium (the device sees a path or a URL, not a diff — nothing there is
/// "low"). The session perimeter (D14) is not known here: anything reaching
/// a host is treated as medium and the keyboard keeps the final word.
pub fn tier_for(tool: &str, input: &Value) -> (&'static str, &'static str, String) {
    if tool == "Bash" {
        let cmd = input.get("command").and_then(Value::as_str).unwrap_or("");
        if let Some((_, why)) = DANGEROUS.iter().find(|(re, _)| re.is_match(cmd)) {
            return ("high", "Irreversible", why.to_string());
        }
        if let Some((_, why)) = CAUTION.iter().find(|(re, _)| re.is_match(cmd)) {
            return if HIGH_REASON.is_match(why) {
                ("high", "Irreversible or external", why.to_string())
            } else {
                ("medium", "Changes the machine", why.to_string())
            };
        }
        if read_only_bash(cmd) {
            return ("low", "Read-only", "inspects, builds or tests".into());
        }
        return (
            "medium",
            "Runs a command",
            "not in the read-only allowlist".into(),
        );
    }
    match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => (
            "medium",
            "Edits a file",
            "the diff is not on the device".into(),
        ),
        "WebFetch" | "WebSearch" => ("medium", "Reaches a host", "outside the machine".into()),
        _ => ("medium", "Uses a tool", format!("{tool} is not classified")),
    }
}

fn cap(s: &str) -> String {
    crate::services::text::cap_chars(s, SUMMARY_CAP)
}

/// What the device sees as the request's one line.
pub fn summary_of(tool: &str, input: &Value) -> String {
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| input.get(k).and_then(Value::as_str))
            .map(cap)
    };
    match tool {
        "Bash" => pick(&["command"]),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "Read" => {
            pick(&["file_path", "path", "notebook_path"])
        }
        "WebFetch" => pick(&["url"]),
        "WebSearch" => pick(&["query"]),
        _ => None,
    }
    .unwrap_or_else(|| {
        let raw = serde_json::to_string(input).unwrap_or_default();
        cap(&raw)
    })
}

/// The ledger's evidence for a session: its last verdict and last check.
pub fn evidence_for(events: &[ProofEvent], term_id: Option<&str>) -> Evidence {
    let Some(t) = term_id else {
        return Evidence::default();
    };
    let mine = || {
        events
            .iter()
            .rev()
            .filter(|e| e.term_id.as_deref() == Some(t))
    };
    let verification = mine()
        .find(|e| e.kind == "verification")
        .and_then(|e| e.payload.get("status").and_then(Value::as_str))
        .map(String::from);
    let last_check = mine().find(|e| e.kind == "check_run").map(|e| LastCheck {
        command: cap(e
            .payload
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")),
        exit_code: e.payload.get("exitCode").and_then(Value::as_i64),
        status: e
            .payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    });
    Evidence {
        verification,
        last_check,
    }
}

/// A pending approval (as the hook wrote it) as the device will see it.
pub fn decision_from(
    req: &Value,
    session: Option<String>,
    events: &[ProofEvent],
) -> RemoteDecision {
    let s = |k: &str| req.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let tool = s("tool");
    let input = req.get("input").cloned().unwrap_or(Value::Null);
    let (tier, label, reason) = tier_for(&tool, &input);
    let term_id = req.get("termId").and_then(Value::as_str).map(String::from);
    RemoteDecision {
        id: s("id"),
        ts: req.get("ts").and_then(Value::as_i64).unwrap_or(0),
        summary: summary_of(&tool, &input),
        tool,
        session,
        project: s("cwd"),
        tier: tier.into(),
        tier_label: label.into(),
        tier_reason: reason,
        remote_allow: tier != "high",
        evidence: evidence_for(events, term_id.as_deref()),
        term_id,
    }
}

/// The gate, in one place: may a device of this scope give this answer to
/// this request? `deny` is always allowed to a device that may decide at
/// all (refusing is never dangerous); `allow` needs the tier to be below
/// high — a dangerous action is decided at the keyboard, whatever the scope.
pub fn gate(scope: DeviceScope, item: &RemoteDecision, decision: &str) -> AppResult<()> {
    if !scope.can_approve_small() {
        return Err(AppError::InvalidArgument(
            "this device may only read — approvals need the readApprovals scope".into(),
        ));
    }
    match decision {
        "deny" => Ok(()),
        "allow" if item.remote_allow => Ok(()),
        "allow" => Err(AppError::InvalidArgument(format!(
            "`{}` is {} ({}): a dangerous action is decided at the keyboard, never from a device",
            item.summary, item.tier_label, item.tier_reason
        ))),
        other => Err(AppError::InvalidArgument(format!(
            "unknown decision `{other}` (allow | deny)"
        ))),
    }
}

/// F0 of the device plan (MOVIL-2026-10): how many decisions never reached
/// this screen. Computed from the ledger, never estimated: an
/// `approval_request` inside the window with no `approval_decision` for its
/// id, and not pending right now, is one the human answered in the CLI or
/// let time out — the console never saw it decided. The wait percentiles
/// are over the decisions the console did take.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DecisionGap {
    pub window_days: u32,
    /// Approval requests the ledger saw in the window.
    pub requests: u64,
    /// Of those, decided through the console (keyboard, device or sponsor).
    pub decided: u64,
    /// Of those, still waiting in the hook's queue right now.
    pub pending: u64,
    /// Of those, neither decided here nor pending: timed out or answered in
    /// the CLI. The number a paired device would have caught.
    pub unseen: u64,
    /// p90 of request → decision, over the console's own decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p90_wait_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wait_ms: Option<i64>,
}

/// The gap over `[since_ms, now]`. `pending` are the ids in the hook's queue
/// at this moment (they are neither unseen nor decided yet). Events outside
/// the window are ignored, except a decision for a request inside it.
pub fn decision_gap(
    events: &[ProofEvent],
    since_ms: i64,
    pending: &std::collections::HashSet<String>,
    window_days: u32,
) -> DecisionGap {
    use std::collections::HashMap;
    let id_of = |e: &ProofEvent| -> Option<String> {
        e.payload
            .get("approvalId")
            .and_then(Value::as_str)
            .map(String::from)
    };
    let mut requested: HashMap<String, i64> = HashMap::new();
    let mut decided_at: HashMap<String, i64> = HashMap::new();
    for e in events {
        match e.kind.as_str() {
            "approval_request" if e.ts >= since_ms => {
                if let Some(id) = id_of(e) {
                    requested.entry(id).or_insert(e.ts);
                }
            }
            "approval_decision" => {
                if let Some(id) = id_of(e) {
                    decided_at.entry(id).or_insert(e.ts);
                }
            }
            _ => {}
        }
    }
    let mut waits: Vec<i64> = Vec::new();
    let mut decided = 0u64;
    let mut pend = 0u64;
    for (id, ts) in &requested {
        if let Some(d) = decided_at.get(id) {
            decided += 1;
            waits.push((d - ts).max(0));
        } else if pending.contains(id) {
            pend += 1;
        }
    }
    waits.sort_unstable();
    let p90 = (!waits.is_empty()).then(|| {
        let idx = ((waits.len() as f64) * 0.9).ceil() as usize;
        waits[idx.clamp(1, waits.len()) - 1]
    });
    let requests = requested.len() as u64;
    DecisionGap {
        window_days,
        requests,
        decided,
        pending: pend,
        unseen: requests - decided - pend,
        p90_wait_ms: p90,
        max_wait_ms: waits.last().copied(),
    }
}

/// MOVIL F1a: one session as a device sees it — enough to know where the
/// agents stand without a chat or a diff. `state` mirrors the Inbox's four
/// groups: `needs_you` (a decision waits), `working` (a turn is open),
/// `idle` (live, nothing open), `stopped` (no terminal behind it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSession {
    pub term_id: String,
    pub name: String,
    pub project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub state: String,
    /// Decisions waiting in this session right now.
    pub pending: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_ms: Option<i64>,
}

/// At most this many sessions in one `sessions` reply (newest activity first
/// within each state).
pub const SESSIONS_CAP: usize = 100;

fn state_rank(state: &str) -> u8 {
    match state {
        "needs_you" => 0,
        "working" => 1,
        "idle" => 2,
        _ => 3,
    }
}

/// What the channel asks of the console: the pending decisions for a device
/// and the recording of one answer. The transport enforces `gate` before
/// `decide` is ever called, so an implementation only has to act.
pub trait DecisionHandler: Send + Sync + 'static {
    /// The decisions waiting, newest first, and whether more wait beyond
    /// `PENDING_CAP`.
    fn pending(&self, device: &PairedDevice) -> AppResult<(Vec<RemoteDecision>, bool)>;
    fn decide(
        &self,
        device: &PairedDevice,
        id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> AppResult<()>;
    /// The open project's sessions and where each stands (read-only; every
    /// scope may ask).
    fn sessions(&self, device: &PairedDevice) -> AppResult<Vec<RemoteSession>>;
    /// MOVIL F1b: the desktop's VAPID public key (base64url), or an error
    /// when the desktop cannot push.
    fn push_info(&self) -> AppResult<String>;
    /// Remember the device's push subscription (any scope).
    fn subscribe(
        &self,
        device: &PairedDevice,
        sub: crate::services::push_service::PushSubscription,
    ) -> AppResult<()>;
    fn unsubscribe(&self, device: &PairedDevice) -> AppResult<()>;
}

/// The part of the bridge that needs no window: the hook's queue in, the
/// hook's answer file and the ledger out. `ConsoleBridge` adds the Tauri
/// state lookup and the `approval://resolved` event on top; tests drive
/// this core against a real `HooksRuntime` and `TestigoService`.
pub struct BridgeCore<'a> {
    pub hooks: &'a crate::services::hooks_service::HooksRuntime,
    pub testigo: &'a crate::services::testigo_service::TestigoService,
    pub sessions: &'a crate::services::sessions_service::SessionsService,
    pub devices: &'a PairedDevicesService,
    /// The open project's root, when any (the ledger-root fallback).
    pub open_project: Option<String>,
}

impl BridgeCore<'_> {
    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn ledger_root(&self, req: &Value) -> String {
        let cwd = req
            .get("cwd")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| self.open_project.clone())
            .unwrap_or_default();
        crate::services::hooks_service::ledger_root_for_cwd(&cwd, self.open_project.as_deref())
    }

    /// Every pending request as the device sees it, newest first, at most
    /// `PENDING_CAP` (the flag says when more wait). The ledger and the
    /// session list are read ONCE per project root, not once per request.
    pub fn pending(&self) -> AppResult<(Vec<RemoteDecision>, bool)> {
        let mut reqs = self.hooks.pending_approvals();
        reqs.sort_by_key(|v| std::cmp::Reverse(v.get("ts").and_then(Value::as_i64).unwrap_or(0)));
        let truncated = reqs.len() > PENDING_CAP;
        reqs.truncate(PENDING_CAP);
        type Cached = (
            Vec<ProofEvent>,
            Vec<crate::services::sessions_service::PersistedSession>,
        );
        let mut by_root: std::collections::HashMap<String, Cached> =
            std::collections::HashMap::new();
        let mut out = Vec::with_capacity(reqs.len());
        for req in &reqs {
            let root = self.ledger_root(req);
            let (events, sessions) = by_root.entry(root.clone()).or_insert_with(|| {
                (
                    self.testigo.list(&root, None, None).unwrap_or_default(),
                    self.sessions.list(&root).unwrap_or_default(),
                )
            });
            let term_id = req.get("termId").and_then(Value::as_str);
            let session =
                term_id.and_then(|t| sessions.iter().find(|s| s.id == t).map(|s| s.name.clone()));
            out.push(decision_from(req, session, events));
        }
        Ok((out, truncated))
    }

    /// Answer one request: the hook's answer file, the ledger line with the
    /// device on it, the device's last-seen stamp. The caller has already run
    /// `gate`. Returns the request's ledger root.
    pub fn decide(
        &self,
        device: &PairedDevice,
        id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> AppResult<String> {
        let req = self
            .hooks
            .pending_approvals()
            .into_iter()
            .find(|r| r.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| AppError::InvalidArgument("that request is no longer pending".into()))?;
        self.hooks.respond(id, decision, reason)?;
        let root = self.ledger_root(&req);
        let _ = self.testigo.on_approval_decision_via(
            &root,
            Self::now_ms(),
            id,
            decision,
            reason,
            None,
            Some(json!({ "remote": { "deviceId": device.id, "label": device.label } })),
        );
        let _ = self.devices.touch(&device.id, Self::now_ms());
        Ok(root)
    }

    /// The open project's sessions as the device shows them. `live` = the
    /// terminal ids that have a shell right now; a decision in the hook's
    /// queue makes `needs_you`, an open ledger turn makes `working`.
    pub fn sessions(
        &self,
        live: &std::collections::HashSet<String>,
    ) -> AppResult<Vec<RemoteSession>> {
        let Some(root) = self.open_project.clone() else {
            return Ok(Vec::new());
        };
        let mut pending: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for req in self.hooks.pending_approvals() {
            if let Some(t) = req.get("termId").and_then(Value::as_str) {
                *pending.entry(t.to_string()).or_default() += 1;
            }
        }
        let mut out: Vec<RemoteSession> = self
            .sessions
            .list(&root)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.archived != Some(true))
            .map(|s| {
                let n = pending.get(&s.id).copied().unwrap_or(0);
                let state = if n > 0 {
                    "needs_you"
                } else if self.testigo.peek_turn(&s.id).is_some() {
                    "working"
                } else if live.contains(&s.id) {
                    "idle"
                } else {
                    "stopped"
                };
                RemoteSession {
                    term_id: s.id,
                    name: s.name,
                    project: root.clone(),
                    agent: s.agent,
                    state: state.into(),
                    pending: n,
                    last_active_ms: s.last_active_ms.map(|v| v as i64),
                }
            })
            .collect();
        out.sort_by(|a, b| {
            state_rank(&a.state)
                .cmp(&state_rank(&b.state))
                .then(b.last_active_ms.cmp(&a.last_active_ms))
        });
        out.truncate(SESSIONS_CAP);
        Ok(out)
    }

    /// Store the device's push subscription on its record and witness it:
    /// a device that can be knocked is part of the exposure the ledger
    /// describes. Only the push service's host goes in the line.
    pub fn subscribe(
        &self,
        device: &PairedDevice,
        sub: crate::services::push_service::PushSubscription,
    ) -> AppResult<()> {
        sub.validate()?;
        let host = sub.host();
        self.devices.set_push(&device.id, Some(sub))?;
        if let Some(root) = &self.open_project {
            let _ = self.testigo.on_remote(
                root,
                Self::now_ms(),
                "remote_device",
                json!({ "action": "push_subscribed", "deviceId": device.id, "label": device.label, "host": host }),
            );
        }
        Ok(())
    }

    pub fn unsubscribe(&self, device: &PairedDevice) -> AppResult<()> {
        self.devices.set_push(&device.id, None)?;
        if let Some(root) = &self.open_project {
            let _ = self.testigo.on_remote(
                root,
                Self::now_ms(),
                "remote_device",
                json!({ "action": "push_unsubscribed", "deviceId": device.id, "label": device.label }),
            );
        }
        Ok(())
    }
}

/// The real bridge: `BridgeCore` over the app's state, plus the Inbox told
/// (`approval://resolved`) so the card leaves the screen with the device's
/// name on it.
pub struct ConsoleBridge {
    app: tauri::AppHandle,
}

impl ConsoleBridge {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    fn with_core<T>(&self, f: impl FnOnce(&BridgeCore<'_>) -> AppResult<T>) -> AppResult<T> {
        use tauri::Manager;
        let state = self.app.state::<crate::state::AppState>();
        let open_project = state
            .inner
            .lock()
            .project
            .as_ref()
            .map(|p| p.root.to_string_lossy().to_string());
        let core = BridgeCore {
            hooks: &state.hooks,
            testigo: &state.testigo,
            sessions: &state.sessions,
            devices: &state.paired_devices,
            open_project,
        };
        f(&core)
    }
}

impl DecisionHandler for ConsoleBridge {
    fn pending(&self, _device: &PairedDevice) -> AppResult<(Vec<RemoteDecision>, bool)> {
        self.with_core(|c| c.pending())
    }

    fn decide(
        &self,
        device: &PairedDevice,
        id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        use tauri::Emitter;
        self.with_core(|c| c.decide(device, id, decision, reason))?;
        let _ = self.app.emit(
            "approval://resolved",
            &json!({ "id": id, "decision": decision, "via": "remote", "device": device.label }),
        );
        Ok(())
    }

    fn sessions(&self, _device: &PairedDevice) -> AppResult<Vec<RemoteSession>> {
        use tauri::Manager;
        let live: std::collections::HashSet<String> = self
            .app
            .state::<crate::state::AppState>()
            .terminals
            .live_shells()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        self.with_core(|c| c.sessions(&live))
    }

    fn push_info(&self) -> AppResult<String> {
        use tauri::Manager;
        self.app
            .state::<crate::state::AppState>()
            .push
            .public_key()
            .ok_or_else(|| {
                AppError::Other(
                    "this desktop has no push key (keystore and file both unavailable)".into(),
                )
            })
    }

    fn subscribe(
        &self,
        device: &PairedDevice,
        sub: crate::services::push_service::PushSubscription,
    ) -> AppResult<()> {
        self.with_core(|c| c.subscribe(device, sub))
    }

    fn unsubscribe(&self, device: &PairedDevice) -> AppResult<()> {
        self.with_core(|c| c.unsubscribe(device))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(scope: DeviceScope) -> PairedDevice {
        PairedDevice {
            id: "dev-1".into(),
            label: "phone".into(),
            public_key: "k".into(),
            scope,
            paired_at_ms: 0,
            last_seen_ms: None,
            push: None,
        }
    }

    fn item(tool: &str, input: Value) -> RemoteDecision {
        decision_from(
            &json!({ "id": "a1", "ts": 5, "tool": tool, "input": input, "cwd": "/p", "termId": "t1" }),
            Some("auth".into()),
            &[],
        )
    }

    fn ev(kind: &str, ts: i64, id: &str) -> ProofEvent {
        ProofEvent {
            seq: 0,
            ts,
            case_id: "term:t1".into(),
            turn_id: None,
            kind: kind.into(),
            term_id: Some("t1".into()),
            session_id: None,
            actor: "agent".into(),
            payload: json!({ "approvalId": id }),
            prev_hash: String::new(),
            hash: String::new(),
        }
    }

    /// The gap counts what the ledger proves: a request with no decision and
    /// not pending now is unseen; the percentiles are over the console's
    /// own decisions; requests before the window do not count.
    #[test]
    fn the_decision_gap_comes_from_the_ledger_not_an_estimate() {
        let events = vec![
            ev("approval_request", 50, "old"), // before the window
            ev("approval_request", 100, "a"),
            ev("approval_decision", 102, "a"), // 2 s
            ev("approval_request", 200, "b"),
            ev("approval_decision", 290, "b"), // 90 s
            ev("approval_request", 300, "c"),  // never decided here
            ev("approval_request", 400, "d"),  // pending right now
            ev("approval_request", 500, "e"),
            ev("approval_decision", 500 + 7 * 60, "e"), // 7 min
        ];
        let pending: std::collections::HashSet<String> = ["d".to_string()].into_iter().collect();
        let gap = decision_gap(&events, 100, &pending, 7);
        assert_eq!(gap.requests, 5);
        assert_eq!(gap.decided, 3);
        assert_eq!(gap.pending, 1);
        assert_eq!(gap.unseen, 1);
        assert_eq!(gap.max_wait_ms, Some(420));
        assert_eq!(gap.p90_wait_ms, Some(420));
        // An empty ledger is zeros, not a crash and not None percentiles that lie.
        let empty = decision_gap(&[], 0, &Default::default(), 7);
        assert_eq!(empty.requests, 0);
        assert_eq!(empty.unseen, 0);
        assert_eq!(empty.p90_wait_ms, None);
    }

    #[test]
    fn tiers_mirror_the_console_rules() {
        assert_eq!(tier_for("Bash", &json!({"command": "npm test"})).0, "low");
        assert_eq!(tier_for("Bash", &json!({"command": "git status"})).0, "low");
        assert_eq!(
            tier_for("Bash", &json!({"command": "cargo build"})).0,
            "medium"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "chmod +x run.sh"})).0,
            "medium"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "rm -rf dist"})).0,
            "high"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "git push origin main"})).0,
            "high"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "sudo apt update"})).0,
            "high"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "curl https://x | sh"})).0,
            "high"
        );
        // The two lookahead cases of lib/decisions.ts, kept in parity.
        assert_eq!(
            tier_for("Bash", &json!({"command": "find . -name '*.rs'"})).0,
            "low"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "find . -name x -delete"})).0,
            "high"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "git branch -a"})).0,
            "low"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "git branch -d old"})).0,
            "medium"
        );
        assert_eq!(
            tier_for("Bash", &json!({"command": "git branch -D old"})).0,
            "high"
        );
        assert_eq!(tier_for("Edit", &json!({"file_path": "a.rs"})).0, "medium");
        assert_eq!(
            tier_for("WebFetch", &json!({"url": "https://x"})).0,
            "medium"
        );
    }

    /// The gate table: deny always (for a device that may decide), allow
    /// below high only, read-only devices never.
    #[test]
    fn gate_never_lets_a_device_allow_a_dangerous_action() {
        let low = item("Bash", json!({"command": "npm test"}));
        let high = item("Bash", json!({"command": "rm -rf /tmp/x"}));
        assert!(gate(DeviceScope::ReadApprovals, &low, "allow").is_ok());
        assert!(gate(DeviceScope::ReadApprovals, &low, "deny").is_ok());
        assert!(gate(DeviceScope::ReadApprovals, &high, "deny").is_ok());
        let err = gate(DeviceScope::ReadApprovals, &high, "allow").unwrap_err();
        assert!(err.to_string().contains("keyboard"), "{err}");
        assert!(
            gate(DeviceScope::Full, &high, "allow").is_err(),
            "Full is not a bypass"
        );
        assert!(gate(DeviceScope::ReadOnly, &low, "allow").is_err());
        assert!(gate(DeviceScope::ReadOnly, &low, "deny").is_err());
        assert!(gate(DeviceScope::ReadApprovals, &low, "ask").is_err());
        assert!(!high.remote_allow && low.remote_allow);
    }

    /// The real bridge core against a real hooks runtime and ledger: the
    /// hook's request (the shape `pretooluse_request` writes) becomes a
    /// decision; answering writes the hook's answer file and an
    /// `approval_decision` carrying the device; the cap and the flag hold.
    #[test]
    fn bridge_core_answers_the_hook_and_the_ledger() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("ac-bridge-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", base.join("data"));
        std::env::set_var("XDG_CACHE_HOME", base.join("cache"));
        let hooks = crate::services::hooks_service::HooksRuntime::new().unwrap();
        let testigo = crate::services::testigo_service::TestigoService::new();
        let sessions = crate::services::sessions_service::SessionsService::new();
        let devices = PairedDevicesService::new();
        let root = base.join("proj").to_string_lossy().to_string();
        std::fs::create_dir_all(&root).unwrap();
        let approvals = hooks.session_dir().join("approvals");
        std::fs::create_dir_all(&approvals).unwrap();
        // Requests as the hook writes them (`pretooluse_request`).
        let write = |id: &str, ts: u64, cmd: &str| {
            std::fs::write(
                approvals.join(format!("{id}.req.json")),
                json!({ "id": id, "ts": ts, "sessionDir": hooks.session_dir(), "cwd": root, "tool": "Bash",
                        "input": { "command": cmd }, "timeoutMs": 90000, "termId": "t1" })
                .to_string(),
            )
            .unwrap();
        };
        write("a-low", 10, "npm test");
        write("a-high", 20, "rm -rf dist");
        let core = BridgeCore {
            hooks: &hooks,
            testigo: &testigo,
            sessions: &sessions,
            devices: &devices,
            open_project: Some(root.clone()),
        };
        let (items, truncated) = core.pending().unwrap();
        assert!(!truncated);
        assert_eq!(
            items.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["a-high", "a-low"],
            "newest first"
        );
        assert_eq!(items[1].tier, "low");
        assert_eq!(items[1].project, root);
        assert_eq!(items[1].term_id.as_deref(), Some("t1"));

        let device = device(DeviceScope::ReadApprovals);
        core.decide(&device, "a-low", "allow", Some("ok")).unwrap();
        let res: Value = serde_json::from_str(
            &std::fs::read_to_string(approvals.join("a-low.res.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(res["decision"], "allow");
        assert_eq!(res["reason"], "ok");
        let ledger = testigo.list(&root, None, None).unwrap();
        let line = ledger
            .iter()
            .rev()
            .find(|e| e.kind == "approval_decision")
            .expect("approval_decision recorded");
        assert_eq!(line.actor, "human");
        assert_eq!(line.payload["via"]["remote"]["label"], "phone");
        assert_eq!(line.payload["approvalId"], "a-low");
        assert!(
            core.decide(&device, "nope", "deny", None).is_err(),
            "an id that is not pending is refused"
        );

        for i in 0..PENDING_CAP {
            write(&format!("x-{i}"), 100 + i as u64, "ls");
        }
        let (items, truncated) = core.pending().unwrap();
        assert_eq!(items.len(), PENDING_CAP);
        assert!(truncated, "more waited than the cap");

        // MOVIL F1a: the sessions as the device sees them. t1 has decisions
        // waiting (needs_you, with the count), t2 has an open turn and a
        // shell (working), t3 has a shell and nothing open (idle), t4 has no
        // shell (stopped); archived ones never show.
        let mk = |id: &str, name: &str, archived: bool, last: u64| {
            crate::services::sessions_service::PersistedSession {
                id: id.into(),
                name: name.into(),
                cwd: root.clone(),
                created_at_ms: 1,
                scrollback: String::new(),
                agent: Some("claude".into()),
                agent_session_id: None,
                name_suggested: None,
                model: None,
                worktree: None,
                agent_id: None,
                runs_in: None,
                allowed_hosts: None,
                archived: archived.then_some(true),
                last_active_ms: Some(last),
            }
        };
        sessions
            .save(
                &root,
                vec![
                    mk("t1", "auth", false, 10),
                    mk("t2", "docs", false, 20),
                    mk("t3", "spike", false, 30),
                    mk("t4", "old", false, 40),
                    mk("t5", "gone", true, 50),
                ],
            )
            .unwrap();
        testigo
            .on_prompt(&root, 5, Some("t2"), None, Some("write docs"), None, None)
            .unwrap();
        let live: std::collections::HashSet<String> =
            ["t1", "t2", "t3"].into_iter().map(String::from).collect();
        let list = core.sessions(&live).unwrap();
        let view: Vec<(&str, &str, u32)> = list
            .iter()
            .map(|s| (s.term_id.as_str(), s.state.as_str(), s.pending))
            .collect();
        assert_eq!(
            view,
            [
                // a-low's req file is still on disk (the hook, not the
                // console, deletes it once answered), so it still counts.
                ("t1", "needs_you", (PENDING_CAP + 2) as u32),
                ("t2", "working", 0),
                ("t3", "idle", 0),
                ("t4", "stopped", 0),
            ]
        );
        assert_eq!(list[0].name, "auth");
        assert_eq!(list[0].project, root);

        // MOVIL F1b: a subscription lands on the device's record and in the
        // ledger with the push service's host only; unsubscribe clears it.
        let pid = devices.propose_pairing("phone", "k-phone", DeviceScope::ReadOnly, 1);
        let real = devices.approve_pairing(&pid, 1).unwrap();
        let sub = crate::services::push_service::PushSubscription {
            endpoint: "https://fcm.googleapis.com/fcm/send/secret-capability".into(),
            p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".into(),
            auth: "BTBZMqHH6r4Tts7J_aSIgg".into(),
        };
        core.subscribe(&real, sub.clone()).unwrap();
        let stored = devices.find_by_key("k-phone").unwrap().unwrap();
        assert_eq!(stored.push, Some(sub));
        let line = testigo
            .list(&root, None, None)
            .unwrap()
            .into_iter()
            .rev()
            .find(|e| e.kind == "remote_device")
            .expect("remote_device line");
        assert_eq!(line.payload["action"], "push_subscribed");
        assert_eq!(line.payload["host"], "fcm.googleapis.com");
        assert!(
            !line.payload.to_string().contains("secret-capability"),
            "the endpoint never reaches the ledger"
        );
        core.unsubscribe(&real).unwrap();
        assert!(devices
            .find_by_key("k-phone")
            .unwrap()
            .unwrap()
            .push
            .is_none());
        let bad = crate::services::push_service::PushSubscription {
            endpoint: "http://plain/".into(),
            ..stored.push.clone().unwrap()
        };
        assert!(core.subscribe(&real, bad).is_err(), "validated at the door");
    }

    /// The device sees the session's last verdict and last check, bounded.
    #[test]
    fn decision_carries_the_ledgers_evidence() {
        let ev = |seq: u64, kind: &str, term: &str, payload: Value| ProofEvent {
            seq,
            ts: seq as i64,
            case_id: format!("term:{term}"),
            turn_id: Some("turn".into()),
            kind: kind.into(),
            term_id: Some(term.into()),
            session_id: None,
            actor: "console".into(),
            payload,
            prev_hash: String::new(),
            hash: String::new(),
        };
        let events = vec![
            ev(
                1,
                "check_run",
                "t1",
                json!({"command": "npm test", "status": "failed", "exitCode": 1}),
            ),
            ev(
                2,
                "verification",
                "t1",
                json!({"status": "claim_contradicted"}),
            ),
            ev(3, "verification", "t2", json!({"status": "verified"})),
        ];
        let d = decision_from(
            &json!({ "id": "a1", "ts": 5, "tool": "Bash", "input": {"command": "x".repeat(900)}, "cwd": "/p", "termId": "t1" }),
            Some("auth".into()),
            &events,
        );
        assert_eq!(
            d.evidence.verification.as_deref(),
            Some("claim_contradicted")
        );
        assert_eq!(
            d.evidence.last_check,
            Some(LastCheck {
                command: "npm test".into(),
                exit_code: Some(1),
                status: "failed".into()
            })
        );
        assert_eq!(d.summary.chars().count(), SUMMARY_CAP + 1);
        assert_eq!(d.session.as_deref(), Some("auth"));
        assert_eq!(evidence_for(&events, None), Evidence::default());
    }
}
