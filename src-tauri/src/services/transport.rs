//! Wire transport for the mobile voice companion (Hito 5).
//!
//! Runs the desktop (responder) side of a connection over any async byte stream
//! — a TCP socket, a WebSocket, or a relay tunnel are all just streams, so this
//! layer is transport-agnostic. It does the length-framing, drives the Noise
//! handshake over the wire (pairing XX+PSK or reconnect KK), and then carries
//! E2E-encrypted application messages.
//!
//! IMPORTANT: this hito deliberately does NOT bind a listening socket. Binding
//! beyond 127.0.0.1 (to the LAN or a relay) is the step that exposes the desktop
//! to the network, which per project policy requires explicit opt-in — that's
//! Hito 6. Here everything is proven over an in-process `tokio::io::duplex`
//! stream, so nothing is exposed while the protocol is validated.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};
use snow::HandshakeState;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::error::{AppError, AppResult};
use crate::services::paired_devices_service::{DeviceScope, PairedDevicesService};
use crate::services::pairing_service::PairingService;
use crate::services::secure_channel::{decrypt, encrypt};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Hard cap on a single frame, so a hostile peer can't make us allocate huge
/// buffers. Generous for handshake messages and short voice utterances.
const MAX_FRAME: usize = 64 * 1024;

/// First (cleartext) message a connecting peer sends to declare intent. Carries
/// no secret — the public key is public and the intent isn't sensitive; the
/// handshake that follows is what authenticates.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Hello {
    /// First contact: complete a QR pairing.
    Pair,
    /// An already-paired device reconnecting, identified by its pinned key.
    Reconnect { key: String },
}

/// What a served connection resulted in.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A pairing handshake completed and is now awaiting human approval.
    PendingApproval { pending_id: String },
    /// A trusted device reconnected and its session ran to completion.
    Reconnected { device_id: String },
}

// ---- Length-prefixed framing ----

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> AppResult<()> {
    if data.len() > MAX_FRAME {
        return Err(AppError::Other(format!("frame too large: {}", data.len())));
    }
    w.write_all(&(data.len() as u32).to_be_bytes()).await?;
    w.write_all(data).await?;
    w.flush().await?;
    Ok(())
}

/// Read one frame. Returns Ok(None) on a clean EOF (peer closed between frames),
/// so callers can distinguish "done" from a mid-frame truncation (an error).
async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> AppResult<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(AppError::Other(format!("frame too large: {n}")));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

async fn read_frame_required<R: AsyncRead + Unpin>(r: &mut R) -> AppResult<Vec<u8>> {
    read_frame(r)
        .await?
        .ok_or_else(|| AppError::Other("unexpected end of stream".into()))
}

/// Drive a Noise handshake to completion over the stream. `initiator` selects who
/// writes first; the pattern (XX or KK) is already baked into the HandshakeState.
async fn handshake_over_stream<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    mut hs: HandshakeState,
    initiator: bool,
) -> AppResult<HandshakeState> {
    let mut buf = vec![0u8; MAX_FRAME];
    let mut my_turn = initiator;
    while !hs.is_handshake_finished() {
        if my_turn {
            let n = hs
                .write_message(&[], &mut buf)
                .map_err(|e| AppError::Other(format!("handshake write: {e}")))?;
            write_frame(stream, &buf[..n]).await?;
        } else {
            let msg = read_frame_required(stream).await?;
            hs.read_message(&msg, &mut buf)
                .map_err(|e| AppError::Other(format!("handshake read: {e}")))?;
        }
        my_turn = !my_turn;
    }
    Ok(hs)
}

// ---- Desktop-side connection handling ----

/// Serve one incoming connection: read its intent, then run the matching
/// handshake + protocol. The desktop is always the responder.
///
/// Takes the stream BY VALUE and owns it for the connection's lifetime: on
/// return — success OR error (e.g. a refused/revoked device) — the stream is
/// dropped, closing it so the peer sees a clean EOF instead of hanging.
pub async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    pairing: &PairingService,
    devices: &PairedDevicesService,
    now_ms: i64,
) -> AppResult<Outcome> {
    let hello_bytes = read_frame_required(&mut stream).await?;
    let hello: Hello = serde_json::from_slice(&hello_bytes)
        .map_err(|e| AppError::Other(format!("bad hello: {e}")))?;
    match hello {
        Hello::Pair => serve_pairing(&mut stream, pairing, devices, now_ms).await,
        Hello::Reconnect { key } => serve_reconnect(&mut stream, &key, pairing, devices).await,
    }
}

/// First-contact pairing (XX+PSK). After the handshake the phone sends its
/// label as the first E2E message; we register a *pending* pairing (trust still
/// requires the human approval in the UI) and acknowledge.
async fn serve_pairing<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    pairing: &PairingService,
    devices: &PairedDevicesService,
    now_ms: i64,
) -> AppResult<Outcome> {
    let hs = pairing.pairing_responder(now_ms)?;
    let hs = handshake_over_stream(stream, hs, false).await?;
    let phone_pub = hs
        .get_remote_static()
        .ok_or_else(|| AppError::Other("no device key after pairing handshake".into()))?
        .to_vec();
    let phone_pub_b64 = BASE64URL_NOPAD.encode(&phone_pub);

    let mut t = hs
        .into_transport_mode()
        .map_err(|e| AppError::Other(format!("transport: {e}")))?;
    let label_ct = read_frame_required(stream).await?;
    let label = String::from_utf8_lossy(&decrypt(&mut t, &label_ct)?).to_string();

    let pending_id = devices.propose_pairing(&label, &phone_pub_b64, DeviceScope::ReadApprovals, now_ms);
    // Tell the phone it's pending the human's approval, then close.
    let ack = encrypt(&mut t, b"pending-approval")?;
    write_frame(stream, &ack).await?;
    Ok(Outcome::PendingApproval { pending_id })
}

/// Reconnect of a paired device (KK). Authenticates against the pinned key in the
/// store, then runs the application message loop (a simple authenticated echo for
/// now — the voice protocol rides on top in a later hito).
async fn serve_reconnect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    key_b64: &str,
    pairing: &PairingService,
    devices: &PairedDevicesService,
) -> AppResult<Outcome> {
    // The auth gate: a revoked/unknown key matches no device → refuse before any
    // crypto work.
    let device = devices
        .find_by_key(key_b64)?
        .ok_or_else(|| AppError::Other("unknown or revoked device".into()))?;
    let device_pub = BASE64URL_NOPAD
        .decode(device.public_key.as_bytes())
        .map_err(|e| AppError::Other(format!("decode device key: {e}")))?;

    let hs = pairing.reconnect_responder(&device_pub)?;
    let hs = handshake_over_stream(stream, hs, false).await?;
    let mut t = hs
        .into_transport_mode()
        .map_err(|e| AppError::Other(format!("transport: {e}")))?;

    // Application loop: decrypt each request, echo an authenticated reply. Ends
    // on a clean EOF. (Voice/commands replace the echo later.)
    while let Some(ct) = read_frame(stream).await? {
        let msg = decrypt(&mut t, &ct)?;
        let mut reply = b"ack:".to_vec();
        reply.extend_from_slice(&msg);
        let out = encrypt(&mut t, &reply)?;
        write_frame(stream, &out).await?;
    }
    Ok(Outcome::Reconnected { device_id: device.id })
}

// ---- Listener ----

/// A running accept loop. Dropping/stopping it aborts the loop; in-flight
/// connection tasks are detached and finish on their own.
pub struct ServerHandle {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl ServerHandle {
    pub fn new(addr: SocketAddr, task: JoinHandle<()>) -> Self {
        Self { addr, task }
    }
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    pub fn stop(self) {
        self.task.abort();
    }
}

/// Accept connections forever on an ALREADY-BOUND listener, serving each on its
/// own task. The caller binds the listener, so the bind address — and thus the
/// exposure decision (loopback vs LAN vs relay) — stays at the call site where
/// it can be policy-gated. `on_done` is invoked with each connection's result
/// (e.g. to notify the UI that a pending pairing arrived).
pub async fn run_listener<F>(
    listener: TcpListener,
    pairing: Arc<PairingService>,
    devices: Arc<PairedDevicesService>,
    on_done: F,
) where
    F: Fn(AppResult<Outcome>) + Send + Sync + Clone + 'static,
{
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => continue,
        };
        let pairing = pairing.clone();
        let devices = devices.clone();
        let on_done = on_done.clone();
        tokio::spawn(async move {
            let out = serve_connection(stream, &pairing, &devices, now_ms()).await;
            on_done(out);
        });
    }
}

#[cfg(test)]
mod tests {
    // The env-serialization guard (lock_env) is intentionally held across awaits
    // for the whole async test, so XDG_DATA_HOME stays isolated while the services
    // do their disk IO. No async contention on it — it only serializes test
    // threads — so holding it across .await is safe here.
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::services::secure_channel::{
        build_initiator, build_initiator_known, generate_static, verify_pinned_remote, PairingOffer,
        StaticKeypair,
    };

    fn isolated_data_dir() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir()
            .join(format!("ac-transport-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
    }

    /// The phone side of the wire protocol, used to drive the desktop transport
    /// over a duplex stream in tests.
    struct PhoneWire {
        keys: StaticKeypair,
        pinned_desktop: Option<Vec<u8>>,
    }

    impl PhoneWire {
        fn new() -> Self {
            Self { keys: generate_static().unwrap(), pinned_desktop: None }
        }

        async fn pair<S: AsyncRead + AsyncWrite + Unpin>(
            &mut self,
            stream: &mut S,
            offer: &PairingOffer,
            label: &str,
        ) -> AppResult<()> {
            self.pinned_desktop = Some(offer.responder_static_bytes()?);
            let hello = serde_json::to_vec(&Hello::Pair).unwrap();
            write_frame(stream, &hello).await?;
            let init = build_initiator(&self.keys.private, &offer.psk_bytes()?)?;
            let hs = handshake_over_stream(stream, init, true).await?;
            verify_pinned_remote(&hs, self.pinned_desktop.as_ref().unwrap())?;
            let mut t = hs.into_transport_mode().map_err(|e| AppError::Other(e.to_string()))?;
            let ct = encrypt(&mut t, label.as_bytes())?;
            write_frame(stream, &ct).await?;
            let ack = decrypt(&mut t, &read_frame_required(stream).await?)?;
            assert_eq!(ack, b"pending-approval");
            Ok(())
        }

        fn public_b64(&self) -> String {
            BASE64URL_NOPAD.encode(&self.keys.public)
        }

        // Takes the stream BY VALUE and drops it on return, so the desktop's
        // read loop sees a clean EOF and finishes — otherwise `join!` would wait
        // forever for a server that's still reading.
        async fn reconnect_echo<S: AsyncRead + AsyncWrite + Unpin>(
            &self,
            mut stream: S,
            msg: &[u8],
        ) -> AppResult<Vec<u8>> {
            let hello = serde_json::to_vec(&Hello::Reconnect { key: self.public_b64() }).unwrap();
            write_frame(&mut stream, &hello).await?;
            let desktop_pub = self.pinned_desktop.as_ref().expect("paired first");
            let init = build_initiator_known(&self.keys.private, desktop_pub)?;
            let hs = handshake_over_stream(&mut stream, init, true).await?;
            let mut t = hs.into_transport_mode().map_err(|e| AppError::Other(e.to_string()))?;
            let ct = encrypt(&mut t, msg)?;
            write_frame(&mut stream, &ct).await?;
            let reply = decrypt(&mut t, &read_frame_required(&mut stream).await?)?;
            Ok(reply)
        }
    }

    #[tokio::test]
    async fn pairing_then_approved_reconnect_over_a_stream() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = 1_700_000_000_000_i64;
        let pairing = PairingService::load().unwrap();
        let devices = PairedDevicesService::new();

        // --- Pair over a real async duplex stream ---
        let offer = pairing.start_pairing(now, 60);
        let (desk, mut phone_side) = tokio::io::duplex(8192);
        let mut phone = PhoneWire::new();
        let phone_b64 = phone.public_b64();

        let server = serve_connection(desk, &pairing, &devices, now);
        let client = phone.pair(&mut phone_side, &offer, "Carlos iPhone");
        let (server_out, client_out) = tokio::join!(server, client);
        client_out.unwrap();
        match server_out.unwrap() {
            Outcome::PendingApproval { .. } => {}
            other => panic!("expected pending approval, got {other:?}"),
        }
        // Pending, not yet trusted.
        assert!(devices.find_by_key(&phone_b64).unwrap().is_none());

        // --- Human approves, then the phone reconnects and exchanges E2E msgs ---
        let pid = devices.pending()[0].id.clone();
        let dev = devices.approve_pairing(&pid, now).unwrap();

        let (desk2, phone2) = tokio::io::duplex(8192);
        let server = serve_connection(desk2, &pairing, &devices, now);
        let client = phone.reconnect_echo(phone2, b"hello agent");
        let (server_out, reply) = tokio::join!(server, client);
        assert_eq!(server_out.unwrap(), Outcome::Reconnected { device_id: dev.id });
        assert_eq!(reply.unwrap(), b"ack:hello agent");
    }

    #[tokio::test]
    async fn revoked_device_cannot_reconnect_over_a_stream() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = 1_700_000_000_000_i64;
        let pairing = PairingService::load().unwrap();
        let devices = PairedDevicesService::new();

        // Pair + approve, then revoke.
        let offer = pairing.start_pairing(now, 60);
        let (desk, mut phone_side) = tokio::io::duplex(8192);
        let mut phone = PhoneWire::new();
        let (s, c) = tokio::join!(
            serve_connection(desk, &pairing, &devices, now),
            phone.pair(&mut phone_side, &offer, "phone")
        );
        c.unwrap();
        s.unwrap();
        let pid = devices.pending()[0].id.clone();
        let dev = devices.approve_pairing(&pid, now).unwrap();
        devices.revoke(&dev.id).unwrap();

        // Reconnect must be refused — the pinned key matches nothing now.
        let (desk2, phone2) = tokio::io::duplex(8192);
        let (server_out, _client) = tokio::join!(
            serve_connection(desk2, &pairing, &devices, now),
            phone.reconnect_echo(phone2, b"let me in")
        );
        assert!(server_out.is_err(), "revoked device must be refused");
    }

    /// End-to-end over a REAL loopback TCP socket (still 127.0.0.1 only — nothing
    /// exposed): the accept loop serves a phone that pairs over the wire, and the
    /// pending pairing shows up in the store.
    #[tokio::test]
    async fn localhost_listener_accepts_a_pairing_over_tcp() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        // Real time, so the offer's TTL matches the listener's now().
        let now = now_ms();
        let pairing = Arc::new(PairingService::load().unwrap());
        let devices = Arc::new(PairedDevicesService::new());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.ip().is_loopback(), "must bind loopback only");
        let offer = pairing.start_pairing(now, 60);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<AppResult<Outcome>>(4);
        let server = tokio::spawn(run_listener(listener, pairing.clone(), devices.clone(), move |out| {
            let _ = tx.try_send(out);
        }));

        // The phone dials in over TCP and pairs.
        let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut phone = PhoneWire::new();
        let phone_b64 = phone.public_b64();
        phone.pair(&mut sock, &offer, "Carlos iPhone").await.unwrap();

        // The accept loop reported a pending pairing for this device.
        let outcome = rx.recv().await.unwrap().unwrap();
        assert!(matches!(outcome, Outcome::PendingApproval { .. }));
        assert_eq!(devices.pending().len(), 1);
        assert_eq!(devices.pending()[0].public_key, phone_b64);

        server.abort();
    }
}
