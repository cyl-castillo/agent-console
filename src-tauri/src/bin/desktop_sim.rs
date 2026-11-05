//! desktop-sim — the DESKTOP end of the remote-decisions protocol, as a
//! standalone binary, for whoever is writing a device app.
//!
//! `phone-sim` is the device; this is the other side: the real listener,
//! the real Noise handshakes, the real trust store and gate, with a canned
//! queue of decisions behind them instead of a running agent. Point a
//! device implementation (the companion PWA, a native app) at it and
//! exercise pair → approve → pending → decide → sessions → push without
//! launching agent-console. The companion repo's end-to-end test drives it.
//!
//! Usage:
//!   desktop-sim [--port N] [--approve-after SECS] [--bind lan]
//!
//! Prints one JSON line per event to stdout (`{"event":"listening",…}`,
//! `offer`, `pending_pairing`, `approved`, `decided`, `subscribed`, …) so a
//! test can follow along; the pairing offer line carries the URI a device
//! scans or pastes. Pairings are auto-approved after `--approve-after`
//! seconds (default 2; `--approve-after 0` = at once) with scope
//! `readApprovals`, the way a human would click on the real desktop.
//!
//! State lives under a throwaway XDG_DATA_HOME in the system temp dir, so
//! nothing here touches a real agent-console install.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_console_lib::device_sim::{
    decision_from, DecisionHandler, DeviceScope, PairedDevice, PairedDevicesService,
    PairingService, PushSubscription, RemoteDecision, RemoteSession,
};
use serde_json::{json, Value};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn emit(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

/// A canned console: three decisions (low, medium, high), two sessions, a
/// push subscription slot. Decisions leave the queue when answered.
struct Canned {
    queue: Mutex<Vec<Value>>,
    subscribed: Mutex<Option<PushSubscription>>,
}

impl Canned {
    fn new() -> Arc<Self> {
        let t = now_ms();
        Arc::new(Self {
            queue: Mutex::new(vec![
                json!({ "id": "ap-low", "ts": t - 40_000, "tool": "Bash", "input": { "command": "npm test" }, "cwd": "/home/u/code/demo", "termId": "sess-auth" }),
                json!({ "id": "ap-medium", "ts": t - 20_000, "tool": "Edit", "input": { "file_path": "src/auth/session.ts", "old_string": "maxAge: 3600", "new_string": "maxAge: 86400" }, "cwd": "/home/u/code/demo", "termId": "sess-auth" }),
                json!({ "id": "ap-high", "ts": t - 5_000, "tool": "Bash", "input": { "command": "git push --force origin main" }, "cwd": "/home/u/code/demo", "termId": "sess-release" }),
            ]),
            subscribed: Mutex::new(None),
        })
    }
}

impl DecisionHandler for Canned {
    fn pending(
        &self,
        _device: &PairedDevice,
    ) -> agent_console_lib::AppResult<(Vec<RemoteDecision>, bool)> {
        let mut reqs = self.queue.lock().unwrap().clone();
        reqs.sort_by_key(|v| std::cmp::Reverse(v.get("ts").and_then(Value::as_i64).unwrap_or(0)));
        let items = reqs
            .iter()
            .map(|r| {
                let session = match r.get("termId").and_then(Value::as_str) {
                    Some("sess-auth") => Some("auth refactor".to_string()),
                    Some("sess-release") => Some("release 2.4".to_string()),
                    _ => None,
                };
                decision_from(r, session, &[])
            })
            .collect();
        Ok((items, false))
    }

    fn decide(
        &self,
        device: &PairedDevice,
        id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> agent_console_lib::AppResult<()> {
        let mut q = self.queue.lock().unwrap();
        let before = q.len();
        q.retain(|r| r.get("id").and_then(Value::as_str) != Some(id));
        if q.len() == before {
            return Err(agent_console_lib::AppError::InvalidArgument(
                "that request is no longer pending".into(),
            ));
        }
        emit(
            json!({ "event": "decided", "id": id, "decision": decision, "reason": reason, "device": device.label }),
        );
        Ok(())
    }

    fn sessions(&self, _device: &PairedDevice) -> agent_console_lib::AppResult<Vec<RemoteSession>> {
        let pending = self.queue.lock().unwrap();
        let count = |term: &str| {
            pending
                .iter()
                .filter(|r| r.get("termId").and_then(Value::as_str) == Some(term))
                .count() as u32
        };
        let mk = |term: &str, name: &str, idle_state: &str, last: i64| {
            let n = count(term);
            RemoteSession {
                term_id: term.into(),
                name: name.into(),
                project: "/home/u/code/demo".into(),
                agent: Some("claude".into()),
                state: if n > 0 {
                    "needs_you".into()
                } else {
                    idle_state.into()
                },
                pending: n,
                last_active_ms: Some(now_ms() - last),
            }
        };
        Ok(vec![
            mk("sess-auth", "auth refactor", "working", 30_000),
            mk("sess-release", "release 2.4", "idle", 600_000),
            RemoteSession {
                term_id: "sess-docs".into(),
                name: "docs sweep".into(),
                project: "/home/u/code/demo".into(),
                agent: Some("codex".into()),
                state: "stopped".into(),
                pending: 0,
                last_active_ms: Some(now_ms() - 86_400_000),
            },
        ])
    }

    fn push_info(&self) -> agent_console_lib::AppResult<String> {
        // RFC 8291 Appendix A's application-server key: a valid P-256 point
        // a browser accepts as `applicationServerKey`. Nothing is ever sent
        // to a push service from here.
        Ok("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8".into())
    }

    fn subscribe(
        &self,
        device: &PairedDevice,
        sub: PushSubscription,
    ) -> agent_console_lib::AppResult<()> {
        sub.validate()?;
        emit(json!({ "event": "subscribed", "device": device.label, "host": sub.host() }));
        *self.subscribed.lock().unwrap() = Some(sub);
        Ok(())
    }

    fn unsubscribe(&self, device: &PairedDevice) -> agent_console_lib::AppResult<()> {
        emit(json!({ "event": "unsubscribed", "device": device.label }));
        *self.subscribed.lock().unwrap() = None;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let port: u16 = flag("--port").map(|p| p.parse()).transpose()?.unwrap_or(0);
    let approve_after: u64 = flag("--approve-after")
        .map(|p| p.parse())
        .transpose()?
        .unwrap_or(2);
    let bind_lan = flag("--bind").as_deref() == Some("lan");

    // A throwaway home: identity, trust store and offers live and die here.
    let base =
        std::env::temp_dir().join(format!("agent-console-desktop-sim-{}", std::process::id()));
    std::fs::create_dir_all(&base)?;
    std::env::set_var("XDG_DATA_HOME", &base);

    let pairing = Arc::new(PairingService::load()?);
    let devices = Arc::new(PairedDevicesService::new());
    let handler = Canned::new();

    let host = if bind_lan {
        local_ip().ok_or("no LAN address")?
    } else {
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    };
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    let addr = listener.local_addr()?;
    emit(
        json!({ "event": "listening", "addr": addr.to_string(), "ws": format!("ws://{addr}/"), "state_dir": base }),
    );

    // One offer for the run (the real desktop mints one per click; a test
    // needs it printed once). 10 minutes, generous for a human at a laptop.
    let offer = pairing.start_pairing(now_ms(), 600);
    let uri = agent_console_lib::phone_protocol::offer_uri(&offer, &addr.to_string());
    emit(json!({ "event": "offer", "uri": uri, "ttlSecs": 600 }));

    // Auto-approve: what the human's click does on the real desktop.
    {
        let devices = devices.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                for p in devices.pending() {
                    emit(json!({ "event": "pending_pairing", "id": p.id, "label": p.label }));
                    tokio::time::sleep(Duration::from_secs(approve_after)).await;
                    if let Ok(d) = devices.approve_pairing(&p.id, now_ms()) {
                        let _ = devices.set_scope(&d.id, DeviceScope::ReadApprovals);
                        emit(
                            json!({ "event": "approved", "id": d.id, "label": d.label, "scope": "readApprovals" }),
                        );
                    }
                }
            }
        });
    }

    agent_console_lib::device_sim::run_listener(
        listener,
        pairing,
        devices,
        handler,
        |out| match out {
            Ok(o) => emit(json!({ "event": "connection", "outcome": format!("{o:?}") })),
            Err(e) => emit(json!({ "event": "connection_error", "error": e.to_string() })),
        },
    )
    .await;
    Ok(())
}

fn local_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:1").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}
