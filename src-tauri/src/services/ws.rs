//! A minimal WebSocket (RFC 6455) layer for the remote-decisions listener.
//!
//! Why it exists: a browser — and so the companion PWA — cannot open a raw
//! TCP socket. The desktop keeps ONE listener; a connection that starts with
//! an HTTP `GET` is upgraded here and then served by exactly the same code
//! as a raw socket (`transport::serve_connection`), Noise and all. The
//! WebSocket carries the protocol's byte stream: binary messages, no text,
//! and message boundaries mean nothing — the length prefix inside the stream
//! is the framing, as on TCP.
//!
//! Scope, on purpose: binary + control frames, no extensions, no
//! subprotocols, no TLS (Noise encrypts above; a `wss://` edge such as
//! Tailscale Serve or a reverse proxy terminates TLS in front of this port
//! when a browser requires it). The subset is small enough to read in one
//! sitting and is covered by tests against the RFC's own vectors.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use data_encoding::BASE64;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

use crate::error::{AppError, AppResult};

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
/// Largest WebSocket message we accept. The protocol's frames are at most
/// 64 KiB plus a prefix; anything bigger is not ours.
const MAX_MESSAGE: usize = 256 * 1024;
/// Largest HTTP upgrade request we read before giving up.
const MAX_HEADER: usize = 8 * 1024;

const OP_CONT: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

/// `Sec-WebSocket-Accept` for a client's `Sec-WebSocket-Key`.
pub fn accept_key(key: &str) -> String {
    let mut h = Sha1::new();
    h.update(key.trim().as_bytes());
    h.update(GUID.as_bytes());
    BASE64.encode(&h.finalize())
}

/// Does a connection that begins with these bytes look like an HTTP upgrade
/// (a browser) rather than the raw framed protocol (whose first bytes are a
/// big-endian length, never ASCII `GET `)?
pub fn looks_like_http(first: &[u8]) -> bool {
    first.starts_with(b"GET ")
}

/// `true` while `first` could still become `GET ` with more bytes.
pub fn could_be_http(first: &[u8]) -> bool {
    first.len() < 4 && b"GET ".starts_with(first)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Reads masked frames, writes unmasked.
    Server,
    /// Reads unmasked frames, writes masked.
    Client,
}

/// Read bytes from `inner` until the end of the HTTP header block. Returns
/// the header text and whatever followed it (the peer may pipeline its first
/// frame behind the handshake).
async fn read_header<S: AsyncRead + Unpin>(inner: &mut S) -> AppResult<(String, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut tmp = [0u8; 1024];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            let head = String::from_utf8(buf)
                .map_err(|_| AppError::Other("websocket handshake is not UTF-8".into()))?;
            return Ok((head, rest));
        }
        if buf.len() > MAX_HEADER {
            return Err(AppError::Other("websocket handshake too large".into()));
        }
        let n = inner.read(&mut tmp).await?;
        if n == 0 {
            return Err(AppError::Other(
                "connection closed during the websocket handshake".into(),
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

/// Header values by lower-cased name; the first line is returned apart.
fn parse_head(head: &str) -> (String, Vec<(String, String)>) {
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("").to_string();
    let headers = lines
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    (first, headers)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn token_present(value: Option<&str>, token: &str) -> bool {
    value
        .map(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
        .unwrap_or(false)
}

/// Server side: consume the HTTP upgrade on `inner`, answer `101`, and hand
/// back a stream that speaks the protocol's bytes. Any path is accepted (the
/// page the PWA was served from decides it); no subprotocol is negotiated.
pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(mut inner: S) -> AppResult<WsStream<S>> {
    let (head, rest) = read_header(&mut inner).await?;
    let (first, headers) = parse_head(&head);
    let mut parts = first.split_whitespace();
    if parts.next() != Some("GET") || !matches!(parts.nth(1), Some("HTTP/1.1")) {
        return Err(AppError::Other(format!(
            "websocket: not an HTTP/1.1 GET: {first}"
        )));
    }
    if !token_present(header(&headers, "upgrade"), "websocket")
        || !token_present(header(&headers, "connection"), "upgrade")
    {
        return Err(AppError::Other("websocket: missing upgrade headers".into()));
    }
    if header(&headers, "sec-websocket-version") != Some("13") {
        return Err(AppError::Other("websocket: version is not 13".into()));
    }
    let key = header(&headers, "sec-websocket-key")
        .ok_or_else(|| AppError::Other("websocket: missing Sec-WebSocket-Key".into()))?;
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    );
    inner.write_all(response.as_bytes()).await?;
    inner.flush().await?;
    Ok(WsStream::new(inner, rest, Role::Server))
}

/// Client side (the reference client and the tests; a real PWA uses the
/// browser's `WebSocket`): send the upgrade for `/` and verify the `101`.
pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut inner: S,
    host: &str,
) -> AppResult<WsStream<S>> {
    let nonce: [u8; 16] = rand::random();
    let key = BASE64.encode(&nonce);
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    inner.write_all(request.as_bytes()).await?;
    inner.flush().await?;
    let (head, rest) = read_header(&mut inner).await?;
    let (first, headers) = parse_head(&head);
    if !first.starts_with("HTTP/1.1 101") {
        return Err(AppError::Other(format!(
            "websocket: server refused the upgrade: {first}"
        )));
    }
    if header(&headers, "sec-websocket-accept") != Some(accept_key(&key).as_str()) {
        return Err(AppError::Other(
            "websocket: Sec-WebSocket-Accept does not match our key".into(),
        ));
    }
    Ok(WsStream::new(inner, rest, Role::Client))
}

/// `(opcode, fin, unmasked payload, input bytes consumed)`.
type Frame = (u8, bool, Vec<u8>, usize);

/// One parsed frame: its opcode and unmasked payload, and how many input
/// bytes it took. `None` = need more bytes.
fn parse_frame(buf: &[u8], role: Role) -> io::Result<Option<Frame>> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    if b0 & 0x70 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "websocket: reserved bits set (no extensions negotiated)",
        ));
    }
    let fin = b0 & 0x80 != 0;
    let opcode = b0 & 0x0f;
    let masked = b1 & 0x80 != 0;
    let mut pos = 2usize;
    let len: usize = match b1 & 0x7f {
        126 => {
            if buf.len() < pos + 2 {
                return Ok(None);
            }
            let n = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
            pos += 2;
            n
        }
        127 => {
            if buf.len() < pos + 8 {
                return Ok(None);
            }
            let mut b = [0u8; 8];
            b.copy_from_slice(&buf[pos..pos + 8]);
            pos += 8;
            let n = u64::from_be_bytes(b);
            if n > MAX_MESSAGE as u64 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "websocket: message too large",
                ));
            }
            n as usize
        }
        n => n as usize,
    };
    if len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "websocket: message too large",
        ));
    }
    match role {
        Role::Server if !masked => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "websocket: client frames must be masked",
            ))
        }
        Role::Client if masked => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "websocket: server frames must not be masked",
            ))
        }
        _ => {}
    }
    let is_control = opcode & 0x8 != 0;
    if is_control && (len > 125 || !fin) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "websocket: malformed control frame",
        ));
    }
    let mask: Option<[u8; 4]> = if masked {
        if buf.len() < pos + 4 {
            return Ok(None);
        }
        let m = [buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]];
        pos += 4;
        Some(m)
    } else {
        None
    };
    if buf.len() < pos + len {
        return Ok(None);
    }
    let mut payload = buf[pos..pos + len].to_vec();
    if let Some(m) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i % 4];
        }
    }
    Ok(Some((opcode, fin, payload, pos + len)))
}

/// Encode one complete (FIN) frame.
fn encode_frame(opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | (opcode & 0x0f));
    let mask_bit = if mask.is_some() { 0x80 } else { 0x00 };
    match payload.len() {
        n if n < 126 => out.push(mask_bit | n as u8),
        n if n <= u16::MAX as usize => {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        Some(m) => {
            out.extend_from_slice(&m);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ m[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

/// A WebSocket connection presented as a byte stream, so the protocol's
/// framing and Noise run over it unchanged. Reads hand back the payload of
/// binary frames in order (fragments and whole messages alike); a close
/// frame or a plain EOF reads as end of stream; pings are answered; a text
/// frame is an error (nothing in this protocol is text). Each `write` becomes
/// one binary frame — the peer must not read anything into message
/// boundaries.
pub struct WsStream<S> {
    inner: S,
    role: Role,
    /// Raw bytes read from `inner`, not yet parsed into frames.
    raw_in: Vec<u8>,
    /// Decoded payload bytes not yet handed to the reader.
    data_out: VecDeque<u8>,
    /// Frames waiting to go out (data frames and control replies), in order.
    wr: VecDeque<u8>,
    /// The peer closed (or the socket ended): reads return EOF from here on.
    closed: bool,
    /// We queued our own close frame (on shutdown or in reply to theirs).
    close_sent: bool,
}

impl<S> WsStream<S> {
    fn new(inner: S, pipelined: Vec<u8>, role: Role) -> Self {
        Self {
            inner,
            role,
            raw_in: pipelined,
            data_out: VecDeque::new(),
            wr: VecDeque::new(),
            closed: false,
            close_sent: false,
        }
    }

    fn mask(&self) -> Option<[u8; 4]> {
        match self.role {
            Role::Server => None,
            Role::Client => Some(rand::random()),
        }
    }

    fn queue_frame(&mut self, opcode: u8, payload: &[u8]) {
        let m = self.mask();
        self.wr.extend(encode_frame(opcode, payload, m));
    }

    fn queue_close(&mut self) {
        if !self.close_sent {
            self.close_sent = true;
            // 1000 = normal closure, big-endian.
            self.queue_frame(OP_CLOSE, &1000u16.to_be_bytes());
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> WsStream<S> {
    /// Push queued output into `inner` as far as it will go right now.
    fn poll_drain(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while !self.wr.is_empty() {
            let (head, _) = self.wr.as_slices();
            let chunk = if head.is_empty() {
                self.wr.make_contiguous();
                self.wr.as_slices().0
            } else {
                head
            };
            match Pin::new(&mut self.inner).poll_write(cx, chunk) {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "websocket: peer stopped reading",
                    )))
                }
                Poll::Ready(Ok(n)) => {
                    self.wr.drain(..n);
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            // Control replies (pong, close) go out opportunistically; a
            // Pending here just means they leave with the next write.
            let _ = this.poll_drain(cx);
            if !this.data_out.is_empty() {
                let n = buf.remaining().min(this.data_out.len());
                for _ in 0..n {
                    // VecDeque has no slice-into-ReadBuf; n is small (≤ 64 KiB).
                    buf.put_slice(&[this.data_out.pop_front().unwrap()]);
                }
                return Poll::Ready(Ok(()));
            }
            if this.closed {
                return Poll::Ready(Ok(()));
            }
            match parse_frame(&this.raw_in, this.role)? {
                Some((opcode, _fin, payload, used)) => {
                    this.raw_in.drain(..used);
                    match opcode {
                        OP_BINARY | OP_CONT => this.data_out.extend(payload),
                        OP_TEXT => {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "websocket: text frames are not part of this protocol",
                            )))
                        }
                        OP_PING => this.queue_frame(OP_PONG, &payload),
                        OP_PONG => {}
                        OP_CLOSE => {
                            this.closed = true;
                            this.queue_close();
                        }
                        other => {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("websocket: unknown opcode {other:#x}"),
                            )))
                        }
                    }
                }
                None => {
                    let mut tmp = [0u8; 8192];
                    let mut rb = ReadBuf::new(&mut tmp);
                    match Pin::new(&mut this.inner).poll_read(cx, &mut rb) {
                        Poll::Ready(Ok(())) => {
                            if rb.filled().is_empty() {
                                // EOF without a close frame: the stream is over.
                                this.closed = true;
                            } else {
                                this.raw_in.extend_from_slice(rb.filled());
                            }
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WsStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        // Bound the buffer: don't accept more until the backlog drained.
        if this.wr.len() > MAX_MESSAGE {
            match this.poll_drain(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        if data.len() > MAX_MESSAGE {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "websocket: write larger than a message",
            )));
        }
        this.queue_frame(OP_BINARY, data);
        // Buffered semantics: the bytes are ours now; `flush` pushes them.
        let _ = this.poll_drain(cx);
        Poll::Ready(Ok(data.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.poll_drain(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.inner).poll_flush(cx),
            other => other,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.queue_close();
        match this.poll_drain(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.inner).poll_shutdown(cx),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    /// RFC 6455 §1.3's worked example.
    #[test]
    fn accept_key_matches_the_rfc_vector() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn sniffing_tells_a_browser_from_the_raw_protocol() {
        assert!(looks_like_http(b"GET / HTTP/1.1\r\n"));
        assert!(!looks_like_http(&[0, 0, 0, 17, b'{']));
        assert!(could_be_http(b"GE"));
        assert!(!could_be_http(&[0, 0]));
        assert!(!could_be_http(b"GET "), "complete: decided, not pending");
    }

    /// RFC 6455 §5.7's masked "Hello" (as a binary frame here) parses to its
    /// payload; the server refuses an unmasked client frame.
    #[test]
    fn frames_follow_the_rfc_examples() {
        let masked = [
            0x82, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
        ];
        let (op, fin, payload, used) = parse_frame(&masked, Role::Server).unwrap().unwrap();
        assert_eq!((op, fin, used), (OP_BINARY, true, masked.len()));
        assert_eq!(payload, b"Hello");
        let unmasked = encode_frame(OP_BINARY, b"Hello", None);
        assert_eq!(unmasked, [0x82, 0x05, b'H', b'e', b'l', b'l', b'o']);
        assert!(parse_frame(&unmasked, Role::Server).is_err());
        let (_, _, p, _) = parse_frame(&unmasked, Role::Client).unwrap().unwrap();
        assert_eq!(p, b"Hello");
        // A 16-bit length and a partial buffer.
        let big = encode_frame(OP_BINARY, &vec![7u8; 300], Some([1, 2, 3, 4]));
        assert_eq!(big[1] & 0x7f, 126);
        assert!(parse_frame(&big[..50], Role::Server).unwrap().is_none());
        let (_, _, p, used) = parse_frame(&big, Role::Server).unwrap().unwrap();
        assert_eq!((p.len(), used), (300, big.len()));
    }

    /// Handshake and a round trip over an in-memory pipe: the client's writes
    /// (masked) arrive as the server's bytes whatever the chunking, a ping is
    /// answered, a text frame is refused, and the server's close ends the
    /// client's read with EOF.
    #[tokio::test]
    async fn a_client_and_a_server_speak_bytes_over_the_upgrade() {
        let (a, b) = duplex(64 * 1024);
        let server = tokio::spawn(async move {
            let mut s = accept(a).await.unwrap();
            let mut got = vec![0u8; 11];
            s.read_exact(&mut got).await.unwrap();
            assert_eq!(&got, b"hello world");
            s.write_all(b"back at you").await.unwrap();
            s.flush().await.unwrap();
            // Reply to the client's ping happens inside read; give it a turn.
            let mut one = [0u8; 1];
            let n = s.read(&mut one).await.unwrap();
            assert_eq!(n, 0, "client closed: EOF");
            s.shutdown().await.unwrap();
        });
        let mut c = connect(b, "127.0.0.1:1").await.unwrap();
        // Two writes → two frames; the server reads one byte stream.
        c.write_all(b"hello ").await.unwrap();
        c.write_all(b"world").await.unwrap();
        c.flush().await.unwrap();
        let mut back = vec![0u8; 11];
        c.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"back at you");
        c.shutdown().await.unwrap();
        let mut one = [0u8; 1];
        assert_eq!(c.read(&mut one).await.unwrap(), 0, "server's close → EOF");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pings_are_answered_and_text_is_refused() {
        let (a, b) = duplex(4096);
        let server = tokio::spawn(async move {
            let mut s = accept(a).await.unwrap();
            let mut got = vec![0u8; 3];
            s.read_exact(&mut got).await.unwrap();
            assert_eq!(&got, b"abc");
            let err = s.read(&mut [0u8; 8]).await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        });
        // A raw client: upgrade, then a ping, a binary frame and a text frame.
        let mut raw = b;
        raw.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").await.unwrap();
        let (head, rest) = read_header(&mut raw).await.unwrap();
        assert!(head.starts_with("HTTP/1.1 101"));
        assert!(head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
        assert!(rest.is_empty());
        let m = [9, 9, 9, 9];
        raw.write_all(&encode_frame(OP_PING, b"hi", Some(m)))
            .await
            .unwrap();
        raw.write_all(&encode_frame(OP_BINARY, b"abc", Some(m)))
            .await
            .unwrap();
        raw.write_all(&encode_frame(OP_TEXT, b"nope", Some(m)))
            .await
            .unwrap();
        raw.flush().await.unwrap();
        // The pong comes back unmasked with the ping's payload.
        let mut pong = [0u8; 4];
        raw.read_exact(&mut pong).await.unwrap();
        assert_eq!(pong, [0x8A, 0x02, b'h', b'i']);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn the_server_refuses_what_is_not_an_upgrade() {
        let (a, b) = duplex(4096);
        let srv = tokio::spawn(async move { accept(a).await.map(|_| ()) });
        let mut raw = b;
        raw.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let err = srv.await.unwrap().unwrap_err().to_string();
        assert!(err.contains("missing upgrade headers"), "{err}");
    }
}
