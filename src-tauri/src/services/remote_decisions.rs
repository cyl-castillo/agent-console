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
use crate::services::paired_devices_service::{DeviceScope, PairedDevice};
use crate::services::testigo_service::ProofEvent;

/// Longest command / path carried to the device.
pub const SUMMARY_CAP: usize = 300;

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

/// The tier the desktop computes for a request before it leaves the machine.
/// Mirrors `lib/decisions.ts` for Bash; every other tool is medium (the
/// device sees a path or a URL, not a diff — nothing there is "low"). The
/// session perimeter (D14) is not known here: anything reaching a host is
/// treated as medium and the keyboard keeps the final word on that too.
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
        if READ_ONLY.is_match(cmd) {
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
    let t = s.trim();
    if t.chars().count() <= SUMMARY_CAP {
        return t.to_string();
    }
    let mut out: String = t.chars().take(SUMMARY_CAP).collect();
    out.push('…');
    out
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

/// What the channel asks of the console: the pending decisions for a device
/// and the recording of one answer. The transport enforces `gate` before
/// `decide` is ever called, so an implementation only has to act.
pub trait DecisionHandler: Send + Sync + 'static {
    fn pending(&self, device: &PairedDevice) -> AppResult<Vec<RemoteDecision>>;
    fn decide(
        &self,
        device: &PairedDevice,
        id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> AppResult<()>;
}

/// The real bridge: the hook's approval queue in, the hook's answer file and
/// the ledger out, the Inbox told (`approval://resolved`) so the card
/// leaves the screen with the device's name on it.
pub struct ConsoleBridge {
    app: tauri::AppHandle,
}

impl ConsoleBridge {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn ledger_root(state: &crate::state::AppState, req: &Value) -> String {
        let open = state
            .inner
            .lock()
            .project
            .as_ref()
            .map(|p| p.root.to_string_lossy().to_string());
        let cwd = req
            .get("cwd")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| open.clone())
            .unwrap_or_default();
        crate::services::hooks_service::ledger_root_for_cwd(&cwd, open.as_deref())
    }
}

impl DecisionHandler for ConsoleBridge {
    fn pending(&self, _device: &PairedDevice) -> AppResult<Vec<RemoteDecision>> {
        use tauri::Manager;
        let state = self.app.state::<crate::state::AppState>();
        let reqs = state.hooks.pending_approvals();
        let mut out = Vec::with_capacity(reqs.len());
        for req in &reqs {
            let root = Self::ledger_root(&state, req);
            let events = state.testigo.list(&root, None, None).unwrap_or_default();
            let term_id = req.get("termId").and_then(Value::as_str);
            let session = term_id.and_then(|t| {
                state
                    .sessions
                    .list(&root)
                    .ok()?
                    .into_iter()
                    .find(|s| s.id == t)
                    .map(|s| s.name)
            });
            out.push(decision_from(req, session, &events));
        }
        Ok(out)
    }

    fn decide(
        &self,
        device: &PairedDevice,
        id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        use tauri::{Emitter, Manager};
        let state = self.app.state::<crate::state::AppState>();
        let req = state
            .hooks
            .pending_approvals()
            .into_iter()
            .find(|r| r.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| AppError::InvalidArgument("that request is no longer pending".into()))?;
        state.hooks.respond(id, decision, reason)?;
        let root = Self::ledger_root(&state, &req);
        let _ = state.testigo.on_approval_decision_via(
            &root,
            Self::now_ms(),
            id,
            decision,
            reason,
            None,
            Some(json!({ "remote": { "deviceId": device.id, "label": device.label } })),
        );
        let _ = state.paired_devices.touch(&device.id, Self::now_ms());
        let _ = self.app.emit(
            "approval://resolved",
            &json!({ "id": id, "decision": decision, "via": "remote", "device": device.label }),
        );
        Ok(())
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
        }
    }

    fn item(tool: &str, input: Value) -> RemoteDecision {
        decision_from(
            &json!({ "id": "a1", "ts": 5, "tool": tool, "input": input, "cwd": "/p", "termId": "t1" }),
            Some("auth".into()),
            &[],
        )
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
