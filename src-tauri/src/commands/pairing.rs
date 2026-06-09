use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::services::paired_devices_service::{DeviceScope, PairedDevice, PendingPairing};
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingStartResult {
    /// Deep link encoding the offer; this is what the QR contains.
    pub uri: String,
    /// Ready-to-render SVG of the QR.
    pub qr_svg: String,
    pub ttl_secs: u64,
    pub expires_at_ms: i64,
}

/// Begin a pairing: mint a one-time offer (pinning the desktop identity) and
/// return it as a QR for the phone to scan.
#[tauri::command]
pub fn pairing_start(state: State<'_, AppState>) -> AppResult<PairingStartResult> {
    let now = now_ms();
    let offer = state.pairing.start_pairing(now, PAIRING_TTL_SECS);
    let json = serde_json::to_string(&offer)
        .map_err(|e| AppError::Other(format!("serialize offer: {e}")))?;
    let uri = format!("agentconsole://pair?d={}", BASE64URL_NOPAD.encode(json.as_bytes()));
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
    let code = QrCode::new(data.as_bytes()).map_err(|e| AppError::Other(format!("qr encode: {e}")))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build())
}

/// Handshakes awaiting the human's approval on the desktop.
#[tauri::command]
pub fn pairing_pending(state: State<'_, AppState>) -> AppResult<Vec<PendingPairing>> {
    Ok(state.paired_devices.pending())
}

/// Approve a pending pairing (the human clicked "pair"), optionally overriding
/// the proposed scope.
#[tauri::command]
pub fn pairing_approve(
    state: State<'_, AppState>,
    pending_id: String,
    scope: Option<DeviceScope>,
) -> AppResult<PairedDevice> {
    let dev = state.paired_devices.approve_pairing(&pending_id, now_ms())?;
    if let Some(s) = scope {
        if s != dev.scope {
            state.paired_devices.set_scope(&dev.id, s)?;
            return state
                .paired_devices
                .find_by_key(&dev.public_key)?
                .ok_or_else(|| AppError::Other("device vanished after scope change".into()));
        }
    }
    Ok(dev)
}

#[tauri::command]
pub fn pairing_reject(state: State<'_, AppState>, pending_id: String) -> AppResult<()> {
    state.paired_devices.reject_pairing(&pending_id);
    Ok(())
}

#[tauri::command]
pub fn devices_list(state: State<'_, AppState>) -> AppResult<Vec<PairedDevice>> {
    state.paired_devices.list_devices()
}

#[tauri::command]
pub fn devices_revoke(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.paired_devices.revoke(&id)
}

#[tauri::command]
pub fn devices_set_scope(
    state: State<'_, AppState>,
    id: String,
    scope: DeviceScope,
) -> AppResult<()> {
    state.paired_devices.set_scope(&id, scope)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceServerStatus {
    pub running: bool,
    pub addr: Option<String>,
}

/// Start the pairing/voice listener bound to **loopback only** (127.0.0.1). This
/// does NOT expose the desktop to the LAN or internet — that is a separate,
/// explicitly policy-gated step. Idempotent: returns the address if already up.
#[tauri::command]
pub async fn voice_server_start(app: AppHandle, state: State<'_, AppState>) -> AppResult<String> {
    if let Some(h) = state.voice_server.lock().unwrap().as_ref() {
        return Ok(h.addr().to_string());
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AppError::Other(format!("bind loopback: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| AppError::Other(format!("local_addr: {e}")))?;
    let pairing = state.pairing.clone();
    let devices = state.paired_devices.clone();
    let task = tokio::spawn(transport::run_listener(listener, pairing, devices, move |_out| {
        // Nudge the UI to refresh its pending/devices lists.
        let _ = app.emit("pairing://changed", ());
    }));
    *state.voice_server.lock().unwrap() = Some(ServerHandle::new(addr, task));
    Ok(addr.to_string())
}

#[tauri::command]
pub fn voice_server_stop(state: State<'_, AppState>) -> AppResult<()> {
    if let Some(h) = state.voice_server.lock().unwrap().take() {
        h.stop();
    }
    Ok(())
}

#[tauri::command]
pub fn voice_server_status(state: State<'_, AppState>) -> AppResult<VoiceServerStatus> {
    let guard = state.voice_server.lock().unwrap();
    Ok(match guard.as_ref() {
        Some(h) => VoiceServerStatus { running: true, addr: Some(h.addr().to_string()) },
        None => VoiceServerStatus { running: false, addr: None },
    })
}

/// DEV ONLY: fabricate a pending pairing so the approval dialog can be exercised
/// before the transport (Hito 5) feeds real handshakes. Generates a throwaway
/// keypair as the "device". Removed once the transport lands.
#[tauri::command]
pub fn pairing_simulate_incoming(state: State<'_, AppState>, label: String) -> AppResult<String> {
    let fake = crate::services::secure_channel::generate_static()?;
    let key = BASE64URL_NOPAD.encode(&fake.public);
    Ok(state
        .paired_devices
        .propose_pairing(&label, &key, DeviceScope::ReadApprovals, now_ms()))
}
