//! Wire transport for the remote-decisions client (F8).
//!
//! Runs the desktop (responder) side of a connection over any async byte
//! stream — a TCP socket, a WebSocket, or a relay tunnel are all just
//! streams, so this layer is transport-agnostic. It does the length-framing,
//! drives the Noise handshake over the wire (pairing XX+PSK or reconnect
//! KK), and then carries E2E-encrypted application messages: the pending
//! decisions out, the device's answers in, each answer gated by
//! `remote_decisions::gate` before the handler ever sees it.
//!
//! Binding a socket is the caller's decision (`run_listener` takes a bound
//! listener): the console binds 127.0.0.1 unless the human chooses the LAN
//! in the UI, and that choice is recorded in the ledger.

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
use crate::services::remote_decisions::{gate, DecisionHandler, RemoteDecision};
use crate::services::secure_channel::{decrypt, encrypt};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Hard cap on a single frame, so a hostile peer can't make us allocate huge
/// buffers. Generous for handshake messages and a page of decisions.
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

// ---- Decisions application protocol (rides on the E2E channel after reconnect) ----

/// A message from the device to the desktop.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Ping,
    /// The decisions waiting for a human, with their evidence.
    Pending,
    /// The human's answer to one of them. `reason` goes into the ledger.
    Decide {
        id: String,
        decision: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// A message from the desktop to the device.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Pong,
    /// Newest first, at most `remote_decisions::PENDING_CAP`; `truncated`
    /// says more wait on the desktop.
    Decisions {
        items: Vec<RemoteDecision>,
        #[serde(default)]
        truncated: bool,
    },
    /// The answer reached the hook and the ledger.
    Decided {
        id: String,
    },
    Error {
        message: String,
    },
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
    handler: Arc<dyn DecisionHandler>,
    now_ms: i64,
) -> AppResult<Outcome> {
    let hello_bytes = read_frame_required(&mut stream).await?;
    let hello: Hello = serde_json::from_slice(&hello_bytes)
        .map_err(|e| AppError::Other(format!("bad hello: {e}")))?;
    match hello {
        Hello::Pair => serve_pairing(&mut stream, pairing, devices, now_ms).await,
        Hello::Reconnect { key } => {
            serve_reconnect(&mut stream, &key, pairing, devices, handler).await
        }
    }
}

/// First-contact pairing (XX+PSK). After the handshake the device sends its
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

    let pending_id =
        devices.propose_pairing(&label, &phone_pub_b64, DeviceScope::ReadApprovals, now_ms);
    // Tell the device it's pending the human's approval, then close.
    let ack = encrypt(&mut t, b"pending-approval")?;
    write_frame(stream, &ack).await?;
    Ok(Outcome::PendingApproval { pending_id })
}

/// Reconnect of a paired device (KK). Authenticates against the pinned key in
/// the store, then runs the decisions loop: each `ClientMessage` is decrypted,
/// gated, dispatched to the handler (blocking work on a blocking thread) and
/// answered with one `ServerMessage`. Ends on a clean EOF.
async fn serve_reconnect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    key_b64: &str,
    pairing: &PairingService,
    devices: &PairedDevicesService,
    handler: Arc<dyn DecisionHandler>,
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

    while let Some(ct) = read_frame(stream).await? {
        let plain = decrypt(&mut t, &ct)?;
        let reply = match serde_json::from_slice::<ClientMessage>(&plain) {
            Ok(ClientMessage::Ping) => ServerMessage::Pong,
            Ok(ClientMessage::Pending) => {
                let h = handler.clone();
                let dev = device.clone();
                match tokio::task::spawn_blocking(move || h.pending(&dev)).await {
                    Ok(Ok((items, truncated))) => ServerMessage::Decisions { items, truncated },
                    Ok(Err(e)) => ServerMessage::Error {
                        message: e.to_string(),
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("handler panicked: {e}"),
                    },
                }
            }
            Ok(ClientMessage::Decide {
                id,
                decision,
                reason,
            }) => {
                let h = handler.clone();
                let dev = device.clone();
                let res = tokio::task::spawn_blocking(move || -> AppResult<String> {
                    // The gate reads the request as the desktop sees it NOW —
                    // never the tier the device claims.
                    let item = h
                        .pending(&dev)?
                        .0
                        .into_iter()
                        .find(|d| d.id == id)
                        .ok_or_else(|| {
                            AppError::InvalidArgument("that request is no longer pending".into())
                        })?;
                    gate(dev.scope, &item, &decision)?;
                    h.decide(&dev, &id, &decision, reason.as_deref())?;
                    Ok(id)
                })
                .await;
                match res {
                    Ok(Ok(id)) => ServerMessage::Decided { id },
                    Ok(Err(e)) => ServerMessage::Error {
                        message: e.to_string(),
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("handler panicked: {e}"),
                    },
                }
            }
            Err(e) => ServerMessage::Error {
                message: format!("bad message: {e}"),
            },
        };
        let bytes = serde_json::to_vec(&reply)
            .map_err(|e| AppError::Other(format!("serialize reply: {e}")))?;
        let out = encrypt(&mut t, &bytes)?;
        write_frame(stream, &out).await?;
    }
    Ok(Outcome::Reconnected {
        device_id: device.id,
    })
}

// ---- Listener ----

/// A running accept loop. Dropping/stopping it aborts the loop; in-flight
/// connection tasks are detached and finish on their own.
pub struct ServerHandle {
    addr: SocketAddr,
    bind: String,
    task: JoinHandle<()>,
}

impl ServerHandle {
    pub fn new(addr: SocketAddr, bind: &str, task: JoinHandle<()>) -> Self {
        Self {
            addr,
            bind: bind.to_string(),
            task,
        }
    }
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    /// `loopback` or `lan` — what the human chose.
    pub fn bind(&self) -> &str {
        &self.bind
    }
    pub fn stop(self) {
        self.task.abort();
    }
}

/// Accept connections forever on an ALREADY-BOUND listener, serving each on its
/// own task. The caller binds the listener, so the bind address — and thus the
/// exposure decision (loopback vs LAN) — stays at the call site where it is
/// policy-gated. `on_done` is invoked with each connection's result (e.g. to
/// tell the UI a pending pairing arrived).
pub async fn run_listener<F>(
    listener: TcpListener,
    pairing: Arc<PairingService>,
    devices: Arc<PairedDevicesService>,
    handler: Arc<dyn DecisionHandler>,
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
        let handler = handler.clone();
        let on_done = on_done.clone();
        tokio::spawn(async move {
            let out = serve_connection(stream, &pairing, &devices, handler, now_ms()).await;
            on_done(out);
        });
    }
}

// ---- Device (initiator) reference client ----

/// The device side of the protocol — the reference implementation a real
/// client (separate repo) mirrors. Production code: the integration tests and
/// the `phone-sim` binary both drive it, and it is the executable spec for
/// `docs/pairing-protocol.md`.
pub mod client {
    use super::{
        decrypt, encrypt, handshake_over_stream, read_frame_required, write_frame, ClientMessage,
        Hello, ServerMessage,
    };
    use crate::error::{AppError, AppResult};
    use crate::services::secure_channel::{
        build_initiator, build_initiator_known, generate_static, verify_pinned_remote,
        PairingOffer, StaticKeypair,
    };
    use data_encoding::BASE64URL_NOPAD;
    use serde::{Deserialize, Serialize};
    use tokio::io::{AsyncRead, AsyncWrite};

    /// Decode an `agentconsole://pair?d=<base64url(json)>[&a=<host:port>]` URI
    /// into its offer and, when present, the listener's address.
    pub fn parse_offer_uri(uri: &str) -> AppResult<(PairingOffer, Option<String>)> {
        let query = uri
            .split_once('?')
            .map(|(_, q)| q)
            .ok_or_else(|| AppError::Other("offer uri has no query".into()))?;
        let mut d = None;
        let mut a = None;
        for part in query.split('&') {
            if let Some(v) = part.strip_prefix("d=") {
                d = Some(v.trim());
            } else if let Some(v) = part.strip_prefix("a=") {
                a = Some(v.trim().to_string());
            }
        }
        let d = d.ok_or_else(|| AppError::Other("offer uri missing d=".into()))?;
        let json = BASE64URL_NOPAD
            .decode(d.as_bytes())
            .map_err(|e| AppError::Other(format!("bad offer encoding: {e}")))?;
        let offer = serde_json::from_slice(&json)
            .map_err(|e| AppError::Other(format!("bad offer json: {e}")))?;
        Ok((offer, a))
    }

    /// A device client: its identity keypair, plus (after pairing) the pinned
    /// desktop key it authenticates against on reconnect.
    pub struct PhoneClient {
        keys: StaticKeypair,
        pinned_desktop: Option<Vec<u8>>,
    }

    impl PhoneClient {
        pub fn new() -> AppResult<Self> {
            Ok(Self {
                keys: generate_static()?,
                pinned_desktop: None,
            })
        }

        pub fn public_b64(&self) -> String {
            BASE64URL_NOPAD.encode(&self.keys.public)
        }

        pub fn is_paired(&self) -> bool {
            self.pinned_desktop.is_some()
        }

        /// First-contact pairing over `stream`. On success, pins the desktop key.
        /// The desktop holds the pairing pending until a human approves it.
        pub async fn pair<S: AsyncRead + AsyncWrite + Unpin>(
            &mut self,
            stream: &mut S,
            offer: &PairingOffer,
            label: &str,
        ) -> AppResult<()> {
            let pinned = offer.responder_static_bytes()?;
            write_frame(stream, &serde_json::to_vec(&Hello::Pair).unwrap()).await?;
            let init = build_initiator(&self.keys.private, &offer.psk_bytes()?)?;
            let hs = handshake_over_stream(stream, init, true).await?;
            // Anti-MITM: the desktop we just spoke to must be the QR's desktop.
            verify_pinned_remote(&hs, &pinned)?;
            let mut t = hs
                .into_transport_mode()
                .map_err(|e| AppError::Other(format!("transport: {e}")))?;
            write_frame(stream, &encrypt(&mut t, label.as_bytes())?).await?;
            let ack = decrypt(&mut t, &read_frame_required(stream).await?)?;
            if ack != b"pending-approval" {
                return Err(AppError::Other("unexpected pairing ack".into()));
            }
            self.pinned_desktop = Some(pinned);
            Ok(())
        }

        /// Reconnect (KK) and send one message, returning the reply. Consumes the
        /// stream so it closes cleanly on return (ending the desktop's loop).
        pub async fn request<S: AsyncRead + AsyncWrite + Unpin>(
            &self,
            mut stream: S,
            msg: ClientMessage,
        ) -> AppResult<ServerMessage> {
            let desktop = self
                .pinned_desktop
                .as_ref()
                .ok_or_else(|| AppError::Other("not paired".into()))?;
            let hello = serde_json::to_vec(&Hello::Reconnect {
                key: self.public_b64(),
            })
            .unwrap();
            write_frame(&mut stream, &hello).await?;
            let init = build_initiator_known(&self.keys.private, desktop)?;
            let hs = handshake_over_stream(&mut stream, init, true).await?;
            let mut t = hs
                .into_transport_mode()
                .map_err(|e| AppError::Other(format!("transport: {e}")))?;
            let bytes = serde_json::to_vec(&msg).map_err(|e| AppError::Other(e.to_string()))?;
            write_frame(&mut stream, &encrypt(&mut t, &bytes)?).await?;
            let reply = decrypt(&mut t, &read_frame_required(&mut stream).await?)?;
            serde_json::from_slice(&reply).map_err(|e| AppError::Other(format!("bad reply: {e}")))
        }

        /// The decisions waiting on the desktop.
        pub async fn pending<S: AsyncRead + AsyncWrite + Unpin>(
            &self,
            stream: S,
        ) -> AppResult<ServerMessage> {
            self.request(stream, ClientMessage::Pending).await
        }

        /// Answer one decision.
        pub async fn decide<S: AsyncRead + AsyncWrite + Unpin>(
            &self,
            stream: S,
            id: &str,
            decision: &str,
            reason: Option<&str>,
        ) -> AppResult<ServerMessage> {
            self.request(
                stream,
                ClientMessage::Decide {
                    id: id.into(),
                    decision: decision.into(),
                    reason: reason.map(String::from),
                },
            )
            .await
        }

        pub fn to_state(&self) -> PhoneState {
            PhoneState {
                private: BASE64URL_NOPAD.encode(&self.keys.private),
                public: BASE64URL_NOPAD.encode(&self.keys.public),
                desktop_public: self
                    .pinned_desktop
                    .as_ref()
                    .map(|d| BASE64URL_NOPAD.encode(d)),
            }
        }

        pub fn from_state(s: &PhoneState) -> AppResult<Self> {
            let dec = |x: &str| {
                BASE64URL_NOPAD
                    .decode(x.as_bytes())
                    .map_err(|e| AppError::Other(format!("decode phone state: {e}")))
            };
            Ok(Self {
                keys: StaticKeypair {
                    private: dec(&s.private)?,
                    public: dec(&s.public)?,
                },
                pinned_desktop: s.desktop_public.as_deref().map(dec).transpose()?,
            })
        }
    }

    /// Persisted device-side state (for the reference CLI). A real app stores
    /// the private key in the OS keystore instead of a file.
    #[derive(Serialize, Deserialize)]
    pub struct PhoneState {
        pub private: String,
        pub public: String,
        pub desktop_public: Option<String>,
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
    use crate::services::paired_devices_service::PairedDevice;
    use crate::services::remote_decisions::decision_from;
    use client::PhoneClient;
    use serde_json::json;
    use std::sync::Mutex;

    fn isolated_data_dir() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "ac-transport-test-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
    }

    /// A handler with two requests in its queue — one low tier, one high —
    /// that records what it was asked to decide.
    struct StubHandler {
        decided: Mutex<Vec<(String, String, String, Option<String>)>>,
    }
    impl StubHandler {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                decided: Mutex::new(Vec::new()),
            })
        }
    }
    impl DecisionHandler for StubHandler {
        fn pending(&self, _device: &PairedDevice) -> AppResult<(Vec<RemoteDecision>, bool)> {
            Ok((
                vec![
                    decision_from(
                        &json!({ "id": "a-low", "ts": 1, "tool": "Bash", "input": {"command": "npm test"}, "cwd": "/p", "termId": "t1" }),
                        Some("auth".into()),
                        &[],
                    ),
                    decision_from(
                        &json!({ "id": "a-high", "ts": 2, "tool": "Bash", "input": {"command": "git push --force"}, "cwd": "/p", "termId": "t1" }),
                        Some("auth".into()),
                        &[],
                    ),
                ],
                false,
            ))
        }
        fn decide(
            &self,
            device: &PairedDevice,
            id: &str,
            decision: &str,
            reason: Option<&str>,
        ) -> AppResult<()> {
            self.decided.lock().unwrap().push((
                device.id.clone(),
                id.into(),
                decision.into(),
                reason.map(String::from),
            ));
            Ok(())
        }
    }

    #[tokio::test]
    async fn pairing_then_approved_reconnect_over_a_stream() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = 1_700_000_000_000_i64;
        let pairing = PairingService::load().unwrap();
        let devices = PairedDevicesService::new();
        let handler = StubHandler::new();

        // --- Pair over a real async duplex stream ---
        let offer = pairing.start_pairing(now, 60);
        let (desk, mut phone_side) = tokio::io::duplex(8192);
        let mut phone = PhoneClient::new().unwrap();
        let phone_b64 = phone.public_b64();

        let server = serve_connection(desk, &pairing, &devices, handler.clone(), now);
        let client = phone.pair(&mut phone_side, &offer, "Carlos iPhone");
        let (server_out, client_out) = tokio::join!(server, client);
        client_out.unwrap();
        match server_out.unwrap() {
            Outcome::PendingApproval { .. } => {}
            other => panic!("expected pending approval, got {other:?}"),
        }
        // Pending, not yet trusted.
        assert!(devices.find_by_key(&phone_b64).unwrap().is_none());

        // --- Human approves, then the device reconnects and reads the queue ---
        let pid = devices.pending()[0].id.clone();
        let dev = devices.approve_pairing(&pid, now).unwrap();

        let (desk2, phone2) = tokio::io::duplex(8192);
        let server = serve_connection(desk2, &pairing, &devices, handler.clone(), now);
        let client = phone.pending(phone2);
        let (server_out, reply) = tokio::join!(server, client);
        assert_eq!(
            server_out.unwrap(),
            Outcome::Reconnected {
                device_id: dev.id.clone()
            }
        );
        match reply.unwrap() {
            ServerMessage::Decisions { items, truncated } => {
                assert!(!truncated);
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].tier, "low");
                assert!(items[0].remote_allow);
                assert_eq!(items[1].tier, "high");
                assert!(!items[1].remote_allow);
            }
            other => panic!("expected decisions, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn revoked_device_cannot_reconnect_over_a_stream() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = 1_700_000_000_000_i64;
        let pairing = PairingService::load().unwrap();
        let devices = PairedDevicesService::new();
        let handler = StubHandler::new();

        // Pair + approve, then revoke.
        let offer = pairing.start_pairing(now, 60);
        let (desk, mut phone_side) = tokio::io::duplex(8192);
        let mut phone = PhoneClient::new().unwrap();
        let (s, c) = tokio::join!(
            serve_connection(desk, &pairing, &devices, handler.clone(), now),
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
            serve_connection(desk2, &pairing, &devices, handler.clone(), now),
            phone.pending(phone2)
        );
        assert!(server_out.is_err(), "revoked device must be refused");
    }

    /// End-to-end over a REAL loopback TCP socket (still 127.0.0.1 only — nothing
    /// exposed): the accept loop serves a device that pairs over the wire, and the
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
        let server = tokio::spawn(run_listener(
            listener,
            pairing.clone(),
            devices.clone(),
            StubHandler::new(),
            move |out| {
                let _ = tx.try_send(out);
            },
        ));

        // The device dials in over TCP and pairs.
        let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut phone = PhoneClient::new().unwrap();
        let phone_b64 = phone.public_b64();
        phone
            .pair(&mut sock, &offer, "Carlos iPhone")
            .await
            .unwrap();

        // The accept loop reported a pending pairing for this device.
        let outcome = rx.recv().await.unwrap().unwrap();
        assert!(matches!(outcome, Outcome::PendingApproval { .. }));
        assert_eq!(devices.pending().len(), 1);
        assert_eq!(devices.pending()[0].public_key, phone_b64);

        server.abort();
    }

    /// F8's gate, end to end over real loopback TCP with the reference client:
    /// pair → approve → reconnect → read the decisions → allow the low one
    /// (recorded, with its reason) → allow the dangerous one (REFUSED: the
    /// keyboard decides those) → deny the dangerous one (recorded). Then a
    /// read-only device may look but not answer. This is the executable
    /// spec the device app mirrors.
    #[tokio::test]
    async fn reference_client_decides_over_tcp_within_the_gate() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = now_ms();
        let pairing = Arc::new(PairingService::load().unwrap());
        let devices = Arc::new(PairedDevicesService::new());
        let handler = StubHandler::new();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let offer = pairing.start_pairing(now, 60);
        let server = tokio::spawn(run_listener(
            listener,
            pairing.clone(),
            devices.clone(),
            handler.clone(),
            move |_out| {},
        ));

        // 1. Pair (the device goes through the same path as `parse_offer_uri`).
        let mut phone = PhoneClient::new().unwrap();
        let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        phone
            .pair(&mut sock, &offer, "Carlos iPhone")
            .await
            .unwrap();
        drop(sock);

        // 2. Human approves on the desktop.
        let pid = devices.pending()[0].id.clone();
        let dev = devices.approve_pairing(&pid, now).unwrap();

        // 3. Reconnect: the queue, with tiers and the remote_allow flag.
        let connect = || tokio::net::TcpStream::connect(addr);
        let reply = phone.pending(connect().await.unwrap()).await.unwrap();
        let items = match reply {
            ServerMessage::Decisions { items, .. } => items,
            other => panic!("expected decisions, got {other:?}"),
        };
        assert_eq!(
            items.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["a-low", "a-high"]
        );

        // 4. Allow the low one: recorded with the device and the reason.
        let r = phone
            .decide(
                connect().await.unwrap(),
                "a-low",
                "allow",
                Some("tests only"),
            )
            .await
            .unwrap();
        assert_eq!(r, ServerMessage::Decided { id: "a-low".into() });

        // 5. Allow the dangerous one: refused by the gate, never reaches the handler.
        let r = phone
            .decide(connect().await.unwrap(), "a-high", "allow", None)
            .await
            .unwrap();
        match r {
            ServerMessage::Error { message } => assert!(message.contains("keyboard"), "{message}"),
            other => panic!("expected the gate to refuse, got {other:?}"),
        }

        // 6. Deny the dangerous one: refusing is always allowed.
        let r = phone
            .decide(connect().await.unwrap(), "a-high", "deny", None)
            .await
            .unwrap();
        assert_eq!(
            r,
            ServerMessage::Decided {
                id: "a-high".into()
            }
        );

        let decided = handler.decided.lock().unwrap().clone();
        assert_eq!(
            decided,
            vec![
                (
                    dev.id.clone(),
                    "a-low".into(),
                    "allow".into(),
                    Some("tests only".into())
                ),
                (dev.id.clone(), "a-high".into(), "deny".into(), None),
            ]
        );

        // 7. A read-only device may look but not answer.
        devices.set_scope(&dev.id, DeviceScope::ReadOnly).unwrap();
        assert!(matches!(
            phone.pending(connect().await.unwrap()).await.unwrap(),
            ServerMessage::Decisions { .. }
        ));
        match phone
            .decide(connect().await.unwrap(), "a-low", "deny", None)
            .await
            .unwrap()
        {
            ServerMessage::Error { message } => assert!(message.contains("only read"), "{message}"),
            other => panic!("expected refusal, got {other:?}"),
        }
        assert_eq!(
            handler.decided.lock().unwrap().len(),
            2,
            "nothing more recorded"
        );

        server.abort();
    }

    #[test]
    fn offer_uri_carries_the_listener_address() {
        let pairing =
            PairingService::new(crate::services::secure_channel::generate_static().unwrap());
        let offer = pairing.start_pairing(1, 60);
        let json = serde_json::to_string(&offer).unwrap();
        let d = BASE64URL_NOPAD.encode(json.as_bytes());
        let (o, a) =
            client::parse_offer_uri(&format!("agentconsole://pair?d={d}&a=192.168.1.9:4870"))
                .unwrap();
        assert_eq!(
            o.responder_static_bytes().unwrap(),
            pairing.identity_public()
        );
        assert_eq!(a.as_deref(), Some("192.168.1.9:4870"));
        let (_, none) = client::parse_offer_uri(&format!("agentconsole://pair?d={d}")).unwrap();
        assert!(none.is_none());
    }
}
