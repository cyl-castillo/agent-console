//! F8: the remote-decisions listener and the devices allowed to answer.
//!
//! Every exposure decision is the human's and goes to the ledger: pairing
//! a device, changing its scope, revoking it, and where the listener binds.
//! The default bind is loopback; the LAN is an explicit choice in the UI
//! (typed confirmation) recorded as `remote_listener {bind: "lan"}`.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::services::paired_devices_service::{DeviceScope, PairedDevice, PendingPairing};
use crate::services::remote_decisions::{ConsoleBridge, DecisionHandler};
use crate::services::secure_channel::IdentityStore;
use crate::services::transport::{self, ServerHandle};
use crate::state::AppState;

/// How long a pairing QR stays valid. Short on purpose — the QR is the
/// out-of-band trust anchor and shouldn't linger on screen.
const PAIRING_TTL_SECS: u64 = 120;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The open project's root, for the ledger lines these acts leave. None when
/// no project is open: the act still happens, the ledger line is skipped.
fn project_root(state: &AppState) -> Option<String> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string())
}

fn witness(state: &AppState, kind: &str, payload: serde_json::Value) {
    if let Some(root) = project_root(state) {
        let _ = state.testigo.on_remote(&root, now_ms(), kind, payload);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub running: bool,
    /// `host:port` a device dials — the LAN address when bound to the LAN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    /// loopback · lan
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    pub identity_store: IdentityStore,
    pub devices: Vec<PairedDevice>,
    pub pending: Vec<PendingPairing>,
}

fn status_of(state: &AppState) -> AppResult<RemoteStatus> {
    let guard = state.remote_server.lock();
    Ok(RemoteStatus {
        running: guard.is_some(),
        addr: guard.as_ref().map(|h| dial_addr(h.addr(), h.bind())),
        bind: guard.as_ref().map(|h| h.bind().to_string()),
        identity_store: state.pairing.identity_store(),
        devices: state.paired_devices.list_devices()?,
        pending: state.paired_devices.pending(),
    })
}

/// The address a device dials: the LAN IP when bound to all interfaces.
fn dial_addr(bound: SocketAddr, bind: &str) -> String {
    if bind == "lan" {
        if let Some(ip) = lan_ip() {
            return format!("{ip}:{}", bound.port());
        }
    }
    bound.to_string()
}

/// The machine's LAN address, found by the route a UDP socket would take
/// (no packet is sent).
fn lan_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:1").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

#[tauri::command(async)]
pub fn remote_status(state: State<'_, AppState>) -> AppResult<RemoteStatus> {
    status_of(&state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingStartResult {
    /// Deep link encoding the offer and the listener's address; the QR's content.
    pub uri: String,
    /// Ready-to-render SVG of the QR.
    pub qr_svg: String,
    pub ttl_secs: u64,
    pub expires_at_ms: i64,
}

/// Begin a pairing: mint a one-time offer (pinning the desktop identity) and
/// return it as a QR for the device to scan. Needs the listener up — an
/// offer nobody can dial is a QR that only expires.
#[tauri::command(async)]
pub fn remote_pairing_start(state: State<'_, AppState>) -> AppResult<PairingStartResult> {
    let addr = {
        let guard = state.remote_server.lock();
        guard
            .as_ref()
            .map(|h| dial_addr(h.addr(), h.bind()))
            .ok_or_else(|| {
                AppError::InvalidArgument("start the listener before pairing a device".into())
            })?
    };
    let now = now_ms();
    let offer = state.pairing.start_pairing(now, PAIRING_TTL_SECS);
    let json = serde_json::to_string(&offer)
        .map_err(|e| AppError::Other(format!("serialize offer: {e}")))?;
    let uri = format!(
        "agentconsole://pair?d={}&a={addr}",
        BASE64URL_NOPAD.encode(json.as_bytes())
    );
    let qr_svg = render_qr_svg(&uri)?;
    Ok(PairingStartResult {
        uri,
        qr_svg,
        ttl_secs: PAIRING_TTL_SECS,
        expires_at_ms: now + (PAIRING_TTL_SECS as i64) * 1000,
    })
}

fn render_qr_svg(data: &str) -> AppResult<String> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let code =
        QrCode::new(data.as_bytes()).map_err(|e| AppError::Other(format!("qr encode: {e}")))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build())
}

/// Handshakes awaiting the human's approval on the desktop.
#[tauri::command(async)]
pub fn remote_pending(state: State<'_, AppState>) -> AppResult<Vec<PendingPairing>> {
    Ok(state.paired_devices.pending())
}

/// Approve a pending pairing (the human clicked), optionally overriding the
/// proposed scope. Witnessed as `remote_device {action: paired}`.
#[tauri::command(async)]
pub fn remote_approve(
    pending_id: String,
    scope: Option<DeviceScope>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<PairedDevice> {
    let mut dev = state
        .paired_devices
        .approve_pairing(&pending_id, now_ms())?;
    if let Some(s) = scope {
        if s != dev.scope {
            state.paired_devices.set_scope(&dev.id, s)?;
            dev = state
                .paired_devices
                .find_by_key(&dev.public_key)?
                .ok_or_else(|| AppError::Other("device vanished after scope change".into()))?;
        }
    }
    witness(
        &state,
        "remote_device",
        json!({ "action": "paired", "deviceId": dev.id, "label": dev.label, "scope": dev.scope, "publicKey": dev.public_key }),
    );
    let _ = app.emit("remote://changed", ());
    Ok(dev)
}

#[tauri::command(async)]
pub fn remote_reject(
    pending_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.paired_devices.reject_pairing(&pending_id);
    let _ = app.emit("remote://changed", ());
    Ok(())
}

#[tauri::command(async)]
pub fn remote_devices(state: State<'_, AppState>) -> AppResult<Vec<PairedDevice>> {
    state.paired_devices.list_devices()
}

/// Revoke = forget the pinned key: the device's next connection is refused
/// before any handshake. Witnessed as `remote_device {action: revoked}`.
#[tauri::command(async)]
pub fn remote_revoke(id: String, app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let label = state
        .paired_devices
        .list_devices()?
        .into_iter()
        .find(|d| d.id == id)
        .map(|d| d.label);
    state.paired_devices.revoke(&id)?;
    witness(
        &state,
        "remote_device",
        json!({ "action": "revoked", "deviceId": id, "label": label }),
    );
    let _ = app.emit("remote://changed", ());
    Ok(())
}

#[tauri::command(async)]
pub fn remote_set_scope(
    id: String,
    scope: DeviceScope,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.paired_devices.set_scope(&id, scope)?;
    witness(
        &state,
        "remote_device",
        json!({ "action": "scope", "deviceId": id, "scope": scope }),
    );
    let _ = app.emit("remote://changed", ());
    Ok(())
}

/// Start the listener. `bind` is `loopback` (127.0.0.1, the default — a
/// device on the same machine or over a tunnel) or `lan` (every interface,
/// the human's explicit choice in the UI). `port` 0 = ephemeral. Idempotent
/// while running with the same bind; a different bind restarts it.
#[tauri::command(async)]
pub async fn remote_server_start(
    bind: Option<String>,
    port: Option<u16>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<RemoteStatus> {
    let bind = bind.unwrap_or_else(|| "loopback".into());
    let host = match bind.as_str() {
        "loopback" => "127.0.0.1",
        "lan" => "0.0.0.0",
        other => {
            return Err(AppError::InvalidArgument(format!(
                "unknown bind `{other}` (loopback | lan)"
            )))
        }
    };
    {
        let mut guard = state.remote_server.lock();
        match guard.as_ref() {
            Some(h) if h.bind() == bind => return status_of(&state),
            Some(_) => {
                if let Some(h) = guard.take() {
                    h.stop();
                }
            }
            None => {}
        }
    }
    let listener = tokio::net::TcpListener::bind((host, port.unwrap_or(0)))
        .await
        .map_err(|e| AppError::Other(format!("bind {host}: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| AppError::Other(format!("local_addr: {e}")))?;
    let pairing = state.pairing.clone();
    let devices = state.paired_devices.clone();
    let handler: Arc<dyn DecisionHandler> = Arc::new(ConsoleBridge::new(app.clone()));
    let notify = app.clone();
    let task = tokio::spawn(transport::run_listener(
        listener,
        pairing,
        devices,
        handler,
        move |_out| {
            // Nudge the UI to refresh its pending/devices lists.
            let _ = notify.emit("remote://changed", ());
        },
    ));
    *state.remote_server.lock() = Some(ServerHandle::new(addr, &bind, task));
    witness(
        &state,
        "remote_listener",
        json!({ "action": "start", "bind": bind, "addr": dial_addr(addr, &bind) }),
    );
    let _ = app.emit("remote://changed", ());
    status_of(&state)
}

#[tauri::command(async)]
pub fn remote_server_stop(app: AppHandle, state: State<'_, AppState>) -> AppResult<RemoteStatus> {
    let stopped = state.remote_server.lock().take();
    if let Some(h) = stopped {
        let bind = h.bind().to_string();
        h.stop();
        witness(
            &state,
            "remote_listener",
            json!({ "action": "stop", "bind": bind }),
        );
    }
    let _ = app.emit("remote://changed", ());
    status_of(&state)
}
