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
use crate::services::remote_decisions::{gate, DecisionHandler, RemoteDecision, RemoteSession};
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
    /// MOVIL F1a: the open project's sessions and where each stands
    /// (read-only; any scope may ask).
    Sessions,
    /// MOVIL F1b: the desktop's VAPID public key, to subscribe with.
    PushInfo,
    /// The browser's PushSubscription: the desktop will knock with
    /// "N decisions wait" and nothing else.
    Subscribe {
        endpoint: String,
        p256dh: String,
        auth: String,
    },
    Unsubscribe,
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
    /// Needs-you first, then working, idle, stopped; at most
    /// `remote_decisions::SESSIONS_CAP`.
    Sessions {
        items: Vec<RemoteSession>,
    },
    /// `applicationServerKey` for `pushManager.subscribe`, base64url.
    PushInfo {
        vapid_public_key: String,
    },
    Subscribed,
    Unsubscribed,
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
#[cfg_attr(not(test), allow(dead_code))]
pub async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    pairing: &PairingService,
    devices: &PairedDevicesService,
    handler: Arc<dyn DecisionHandler>,
    now_ms: i64,
) -> AppResult<Outcome> {
    serve_connection_with(
        stream,
        pairing,
        devices,
        handler,
        now_ms,
        &Limits::default(),
    )
    .await
}

/// Bounds on one connection (SEGURIDAD-MOVIL F4, M3): an unauthenticated
/// peer gets `setup` to say Hello and finish the handshake; an authenticated
/// one may stay quiet `idle` between requests. Tests pass small values.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub setup: std::time::Duration,
    pub idle: std::time::Duration,
    /// Connections served at once; the rest wait in the accept queue.
    pub concurrent: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            setup: std::time::Duration::from_secs(10),
            idle: std::time::Duration::from_secs(5 * 60),
            concurrent: 32,
        }
    }
}

fn timed_out(what: &str) -> AppError {
    AppError::Other(format!("{what}: the peer did not finish in time"))
}

pub async fn serve_connection_with<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    pairing: &PairingService,
    devices: &PairedDevicesService,
    handler: Arc<dyn DecisionHandler>,
    now_ms: i64,
    limits: &Limits,
) -> AppResult<Outcome> {
    let hello_bytes = tokio::time::timeout(limits.setup, read_frame_required(&mut stream))
        .await
        .map_err(|_| timed_out("hello"))??;
    let hello: Hello = serde_json::from_slice(&hello_bytes)
        .map_err(|e| AppError::Other(format!("bad hello: {e}")))?;
    match hello {
        Hello::Pair => tokio::time::timeout(
            limits.setup,
            serve_pairing(&mut stream, pairing, devices, now_ms),
        )
        .await
        .map_err(|_| timed_out("pairing"))?,
        Hello::Reconnect { key } => {
            serve_reconnect(&mut stream, &key, pairing, devices, handler, limits).await
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
    let (hs, generation) = pairing.pairing_responder(now_ms)?;
    let hs = handshake_over_stream(stream, hs, false).await?;
    // Only a peer that held the PSK gets here: the one-time offer is spent
    // now, not when someone merely said "pair".
    pairing.consume_offer(generation);
    let phone_pub = hs
        .get_remote_static()
        .ok_or_else(|| AppError::Other("no device key after pairing handshake".into()))?
        .to_vec();
    let phone_pub_b64 = BASE64URL_NOPAD.encode(&phone_pub);
    // The short authentication string: both ends derive it from the
    // handshake transcript, the human compares the screens (F6, M4).
    let sas = crate::services::sas::sas_from_hash(hs.get_handshake_hash());

    let mut t = hs
        .into_transport_mode()
        .map_err(|e| AppError::Other(format!("transport: {e}")))?;
    let label_ct = read_frame_required(stream).await?;
    let label = String::from_utf8_lossy(&decrypt(&mut t, &label_ct)?).to_string();

    let pending_id = devices.propose_pairing_with_sas(
        &label,
        &phone_pub_b64,
        DeviceScope::ReadApprovals,
        now_ms,
        &sas,
    );
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
    limits: &Limits,
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
    let hs = tokio::time::timeout(limits.setup, handshake_over_stream(stream, hs, false))
        .await
        .map_err(|_| timed_out("handshake"))??;
    let mut t = hs
        .into_transport_mode()
        .map_err(|e| AppError::Other(format!("transport: {e}")))?;

    loop {
        let Some(ct) = tokio::time::timeout(limits.idle, read_frame(stream))
            .await
            .map_err(|_| timed_out("request"))??
        else {
            break;
        };
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
                    let item = h.lookup(&dev, &id)?.ok_or_else(|| {
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
            Ok(ClientMessage::Sessions) => {
                let h = handler.clone();
                let dev = device.clone();
                match tokio::task::spawn_blocking(move || h.sessions(&dev)).await {
                    Ok(Ok(items)) => ServerMessage::Sessions { items },
                    Ok(Err(e)) => ServerMessage::Error {
                        message: e.to_string(),
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("handler panicked: {e}"),
                    },
                }
            }
            Ok(ClientMessage::PushInfo) => {
                let h = handler.clone();
                match tokio::task::spawn_blocking(move || h.push_info()).await {
                    Ok(Ok(vapid_public_key)) => ServerMessage::PushInfo { vapid_public_key },
                    Ok(Err(e)) => ServerMessage::Error {
                        message: e.to_string(),
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("handler panicked: {e}"),
                    },
                }
            }
            Ok(ClientMessage::Subscribe {
                endpoint,
                p256dh,
                auth,
            }) => {
                let h = handler.clone();
                let dev = device.clone();
                let sub = crate::services::push_service::PushSubscription {
                    endpoint,
                    p256dh,
                    auth,
                };
                match tokio::task::spawn_blocking(move || h.subscribe(&dev, sub)).await {
                    Ok(Ok(())) => ServerMessage::Subscribed,
                    Ok(Err(e)) => ServerMessage::Error {
                        message: e.to_string(),
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("handler panicked: {e}"),
                    },
                }
            }
            Ok(ClientMessage::Unsubscribe) => {
                let h = handler.clone();
                let dev = device.clone();
                match tokio::task::spawn_blocking(move || h.unsubscribe(&dev)).await {
                    Ok(Ok(())) => ServerMessage::Unsubscribed,
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
    run_listener_with(
        listener,
        pairing,
        devices,
        handler,
        on_done,
        Limits::default(),
    )
    .await
}

pub async fn run_listener_with<F>(
    listener: TcpListener,
    pairing: Arc<PairingService>,
    devices: Arc<PairedDevicesService>,
    handler: Arc<dyn DecisionHandler>,
    on_done: F,
    limits: Limits,
) where
    F: Fn(AppResult<Outcome>) + Send + Sync + Clone + 'static,
{
    // At most `concurrent` connections in flight: a flood waits in the
    // kernel's accept queue instead of spawning a task per socket.
    let slots = Arc::new(tokio::sync::Semaphore::new(limits.concurrent.max(1)));
    loop {
        let permit = match slots.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };
        let (stream, _peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => {
                // EMFILE and friends: back off instead of spinning hot.
                drop(permit);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        let pairing = pairing.clone();
        let devices = devices.clone();
        let handler = handler.clone();
        let on_done = on_done.clone();
        tokio::spawn(async move {
            let out = serve_sniffed(stream, &pairing, &devices, handler, &limits).await;
            drop(permit);
            on_done(out);
        });
    }
}

/// One listener, two transports: read the first four bytes (bounded by
/// `limits.setup`); an HTTP `GET` is a browser (the PWA) and is upgraded to
/// a WebSocket, anything else is the raw framed protocol. The bytes read
/// are handed back to whichever transport runs, so nothing is lost. Both
/// then run the same `serve_connection` — same Hello, same Noise, same gate.
async fn serve_sniffed(
    mut stream: tokio::net::TcpStream,
    pairing: &PairingService,
    devices: &PairedDevicesService,
    handler: Arc<dyn DecisionHandler>,
    limits: &Limits,
) -> AppResult<Outcome> {
    use tokio::io::AsyncReadExt;
    let mut first = Vec::with_capacity(4);
    let read_prefix = async {
        let mut byte = [0u8; 1];
        while first.len() < 4 {
            let n = stream.read(&mut byte).await?;
            if n == 0 {
                break;
            }
            first.push(byte[0]);
            if !crate::services::ws::could_be_http(&first) {
                break;
            }
        }
        Ok::<(), std::io::Error>(())
    };
    tokio::time::timeout(limits.setup, read_prefix)
        .await
        .map_err(|_| timed_out("first bytes"))??;
    if first.is_empty() {
        return Err(AppError::Other("the peer sent nothing".into()));
    }
    if crate::services::ws::looks_like_http(&first) {
        let ws = tokio::time::timeout(
            limits.setup,
            crate::services::ws::accept_with_prefix(stream, first),
        )
        .await
        .map_err(|_| timed_out("websocket upgrade"))??;
        serve_connection_with(ws, pairing, devices, handler, now_ms(), limits).await
    } else {
        let prefixed = Prefixed {
            prefix: first,
            pos: 0,
            inner: stream,
        };
        serve_connection_with(prefixed, pairing, devices, handler, now_ms(), limits).await
    }
}

/// A stream whose first bytes were already read by the sniffer: they are
/// replayed before the inner stream is consulted; writes pass through.
struct Prefixed<S> {
    prefix: Vec<u8>,
    pos: usize,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<S> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.pos < this.prefix.len() {
            let n = buf.remaining().min(this.prefix.len() - this.pos);
            buf.put_slice(&this.prefix[this.pos..this.pos + n]);
            this.pos += n;
            return std::task::Poll::Ready(Ok(()));
        }
        std::pin::Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, data)
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
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

    /// Encode an offer as the QR's deep link: `agentconsole://pair?d=<base64url(json)>&a=<host:port>`.
    pub fn offer_uri(offer: &PairingOffer, addr: &str) -> String {
        let json = serde_json::to_string(offer).unwrap_or_default();
        format!(
            "agentconsole://pair?d={}&a={addr}",
            BASE64URL_NOPAD.encode(json.as_bytes())
        )
    }

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

        /// First-contact pairing over `stream`. On success, pins the desktop key
        /// and returns the short authentication string the desktop shows on
        /// the pending pairing — show it; the human compares. The desktop
        /// holds the pairing pending until a human approves it.
        pub async fn pair<S: AsyncRead + AsyncWrite + Unpin>(
            &mut self,
            stream: &mut S,
            offer: &PairingOffer,
            label: &str,
        ) -> AppResult<String> {
            let pinned = offer.responder_static_bytes()?;
            write_frame(stream, &serde_json::to_vec(&Hello::Pair).unwrap()).await?;
            let init = build_initiator(&self.keys.private, &offer.psk_bytes()?)?;
            let hs = handshake_over_stream(stream, init, true).await?;
            // Anti-MITM: the desktop we just spoke to must be the QR's desktop.
            verify_pinned_remote(&hs, &pinned)?;
            let sas = crate::services::sas::sas_from_hash(hs.get_handshake_hash());
            let mut t = hs
                .into_transport_mode()
                .map_err(|e| AppError::Other(format!("transport: {e}")))?;
            write_frame(stream, &encrypt(&mut t, label.as_bytes())?).await?;
            let ack = decrypt(&mut t, &read_frame_required(stream).await?)?;
            if ack != b"pending-approval" {
                return Err(AppError::Other("unexpected pairing ack".into()));
            }
            self.pinned_desktop = Some(pinned);
            Ok(sas)
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

        /// The open project's sessions and where each stands.
        pub async fn sessions<S: AsyncRead + AsyncWrite + Unpin>(
            &self,
            stream: S,
        ) -> AppResult<ServerMessage> {
            self.request(stream, ClientMessage::Sessions).await
        }

        /// The key to subscribe with.
        pub async fn push_info<S: AsyncRead + AsyncWrite + Unpin>(
            &self,
            stream: S,
        ) -> AppResult<ServerMessage> {
            self.request(stream, ClientMessage::PushInfo).await
        }

        /// Hand the desktop a push subscription.
        pub async fn subscribe<S: AsyncRead + AsyncWrite + Unpin>(
            &self,
            stream: S,
            endpoint: &str,
            p256dh: &str,
            auth: &str,
        ) -> AppResult<ServerMessage> {
            self.request(
                stream,
                ClientMessage::Subscribe {
                    endpoint: endpoint.into(),
                    p256dh: p256dh.into(),
                    auth: auth.into(),
                },
            )
            .await
        }

        pub async fn unsubscribe<S: AsyncRead + AsyncWrite + Unpin>(
            &self,
            stream: S,
        ) -> AppResult<ServerMessage> {
            self.request(stream, ClientMessage::Unsubscribe).await
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
        subscribed: Mutex<Option<crate::services::push_service::PushSubscription>>,
    }
    impl StubHandler {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                decided: Mutex::new(Vec::new()),
                subscribed: Mutex::new(None),
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
        fn sessions(&self, _device: &PairedDevice) -> AppResult<Vec<RemoteSession>> {
            Ok(vec![RemoteSession {
                term_id: "t1".into(),
                name: "auth".into(),
                project: "/p".into(),
                agent: Some("claude".into()),
                state: "needs_you".into(),
                pending: 2,
                last_active_ms: Some(7),
            }])
        }
        fn push_info(&self) -> AppResult<String> {
            Ok("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8".into())
        }
        fn subscribe(
            &self,
            _device: &PairedDevice,
            sub: crate::services::push_service::PushSubscription,
        ) -> AppResult<()> {
            sub.validate()?;
            *self.subscribed.lock().unwrap() = Some(sub);
            Ok(())
        }
        fn unsubscribe(&self, _device: &PairedDevice) -> AppResult<()> {
            *self.subscribed.lock().unwrap() = None;
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
        let sas_on_phone = phone
            .pair(&mut sock, &offer, "Carlos iPhone")
            .await
            .unwrap();

        // The accept loop reported a pending pairing for this device.
        let outcome = rx.recv().await.unwrap().unwrap();
        assert!(matches!(outcome, Outcome::PendingApproval { .. }));
        assert_eq!(devices.pending().len(), 1);
        assert_eq!(devices.pending()[0].public_key, phone_b64);
        // F6: both screens show the same four words.
        let shown = devices.pending()[0].sas.clone();
        assert_eq!(shown.split('-').count(), 4, "{shown}");
        assert_eq!(shown, sas_on_phone);

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

    /// MOVIL F1a: the same listener serves a browser. The reference client
    /// dials the TCP port, upgrades to a WebSocket, and pairs, reconnects,
    /// reads the decisions and the sessions, and decides — same Hello, same
    /// Noise, same gate as the raw socket, because it IS the same code.
    #[tokio::test]
    async fn a_websocket_client_pairs_and_decides_through_the_same_listener() {
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
        let dial = || async {
            let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
            crate::services::ws::connect(tcp, &addr.to_string())
                .await
                .unwrap()
        };

        let mut phone = PhoneClient::new().unwrap();
        let mut ws = dial().await;
        phone
            .pair(&mut ws, &offer, "Pixel (browser)")
            .await
            .unwrap();
        drop(ws);
        let pid = devices.pending()[0].id.clone();
        devices.approve_pairing(&pid, now).unwrap();

        let reply = phone.pending(dial().await).await.unwrap();
        let ServerMessage::Decisions { items, .. } = reply else {
            panic!("expected decisions, got {reply:?}");
        };
        assert_eq!(items.len(), 2);
        let reply = phone.sessions(dial().await).await.unwrap();
        assert_eq!(
            reply,
            ServerMessage::Sessions {
                items: vec![RemoteSession {
                    term_id: "t1".into(),
                    name: "auth".into(),
                    project: "/p".into(),
                    agent: Some("claude".into()),
                    state: "needs_you".into(),
                    pending: 2,
                    last_active_ms: Some(7),
                }]
            }
        );
        let reply = phone
            .decide(dial().await, "a-low", "allow", Some("from a browser"))
            .await
            .unwrap();
        assert_eq!(reply, ServerMessage::Decided { id: "a-low".into() });
        let reply = phone
            .decide(dial().await, "a-high", "allow", None)
            .await
            .unwrap();
        assert!(
            matches!(reply, ServerMessage::Error { .. }),
            "the gate holds over a websocket too: {reply:?}"
        );
        let decided = handler.decided.lock().unwrap();
        assert_eq!(decided.len(), 1);
        assert_eq!(decided[0].1, "a-low");
        drop(decided);

        // MOVIL F1b: the browser asks for the key, subscribes, and the
        // desktop keeps the subscription; an http endpoint is refused.
        let reply = phone.push_info(dial().await).await.unwrap();
        let ServerMessage::PushInfo { vapid_public_key } = reply else {
            panic!("expected push info, got {reply:?}");
        };
        assert_eq!(
            BASE64URL_NOPAD
                .decode(vapid_public_key.as_bytes())
                .unwrap()
                .len(),
            65
        );
        let reply = phone
            .subscribe(
                dial().await,
                "https://fcm.googleapis.com/fcm/send/abc",
                "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
                "BTBZMqHH6r4Tts7J_aSIgg",
            )
            .await
            .unwrap();
        assert_eq!(reply, ServerMessage::Subscribed);
        assert_eq!(
            handler
                .subscribed
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.host()),
            Some("fcm.googleapis.com".into())
        );
        let reply = phone
            .subscribe(dial().await, "http://plain/", "AAAA", "AAAA")
            .await
            .unwrap();
        assert!(matches!(reply, ServerMessage::Error { .. }));
        assert_eq!(
            phone.unsubscribe(dial().await).await.unwrap(),
            ServerMessage::Unsubscribed
        );
        assert!(handler.subscribed.lock().unwrap().is_none());

        // And a raw socket still works beside it on the same port.
        let raw = tokio::net::TcpStream::connect(addr).await.unwrap();
        let reply = phone.pending(raw).await.unwrap();
        assert!(matches!(reply, ServerMessage::Decisions { .. }));
        server.abort();
    }

    /// The QR's deep link round-trips through the reference parser — the
    /// desktop, phone-sim and desktop-sim all build it with this one helper.
    /// SEGURIDAD-MOVIL F4 (M3): a peer that says "pair" without the PSK —
    /// anyone who can reach the port — does not burn the QR; the real phone
    /// still pairs with the same offer afterwards.
    #[tokio::test]
    async fn a_psk_less_pair_attempt_does_not_spend_the_offer() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = now_ms();
        let pairing = Arc::new(PairingService::load().unwrap());
        let devices = Arc::new(PairedDevicesService::new());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let offer = pairing.start_pairing(now, 60);
        let server = tokio::spawn(run_listener_with(
            listener,
            pairing.clone(),
            devices.clone(),
            StubHandler::new(),
            move |_out| {},
            Limits {
                setup: std::time::Duration::from_secs(2),
                ..Limits::default()
            },
        ));
        // The attacker: a pair Hello and a handshake with the wrong PSK.
        let mut wrong = offer.clone();
        wrong.psk = BASE64URL_NOPAD.encode(&[7u8; 32]);
        let mut impostor = PhoneClient::new().unwrap();
        let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        assert!(impostor.pair(&mut sock, &wrong, "junk").await.is_err());
        drop(sock);
        // And one that only says hello, then hangs: closed by the setup bound.
        let mut hang = tokio::net::TcpStream::connect(addr).await.unwrap();
        write_frame(&mut hang, br#"{"type":"pair"}"#).await.unwrap();
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut b = [0u8; 1];
            hang.read(&mut b).await
        })
        .await
        .expect("the desktop closes an idle unauthenticated peer");
        assert!(matches!(closed, Ok(0) | Err(_)));
        // The legitimate phone pairs with the very same offer.
        let mut phone = PhoneClient::new().unwrap();
        let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        phone
            .pair(&mut sock, &offer, "Carlos iPhone")
            .await
            .unwrap();
        assert_eq!(devices.pending().len(), 1);
        // And now it is spent: a second completion with it is refused.
        let mut again = PhoneClient::new().unwrap();
        let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        assert!(again.pair(&mut sock, &offer, "replay").await.is_err());
        server.abort();
    }

    /// F4 (M3): the listener serves at most `concurrent` connections at
    /// once; a flood of silent sockets waits in the accept queue and the
    /// legitimate client is still served within the setup bound.
    #[tokio::test]
    async fn a_flood_of_silent_sockets_does_not_starve_a_real_client() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = now_ms();
        let pairing = Arc::new(PairingService::load().unwrap());
        let devices = Arc::new(PairedDevicesService::new());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let offer = pairing.start_pairing(now, 60);
        let server = tokio::spawn(run_listener_with(
            listener,
            pairing.clone(),
            devices.clone(),
            StubHandler::new(),
            move |_out| {},
            Limits {
                setup: std::time::Duration::from_millis(500),
                idle: std::time::Duration::from_secs(5),
                concurrent: 4,
            },
        ));
        let mut flood = Vec::new();
        for _ in 0..40 {
            if let Ok(s) = tokio::net::TcpStream::connect(addr).await {
                flood.push(s); // silent: never sends a byte
            }
        }
        let started = std::time::Instant::now();
        let mut phone = PhoneClient::new().unwrap();
        let mut sock = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            phone.pair(&mut sock, &offer, "Carlos iPhone"),
        )
        .await
        .expect("served despite the flood")
        .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(devices.pending().len(), 1);
        drop(flood);
        server.abort();
    }

    #[test]
    fn offer_uri_round_trips_through_the_parser() {
        let kp = crate::services::secure_channel::generate_static().unwrap();
        let (offer, psk) =
            crate::services::secure_channel::new_offer(&kp.public, 1_700_000_000_000, 120);
        let uri = client::offer_uri(&offer, "192.168.1.9:4870");
        assert!(uri.starts_with("agentconsole://pair?d="));
        let (parsed, addr) = client::parse_offer_uri(&uri).unwrap();
        assert_eq!(addr.as_deref(), Some("192.168.1.9:4870"));
        assert_eq!(parsed.responder_static_bytes().unwrap(), kp.public);
        assert_eq!(parsed.psk_bytes().unwrap(), psk);
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
