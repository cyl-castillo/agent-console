//! Secure pairing + end-to-end channel for the mobile voice companion.
//!
//! This is the security core (Hito 1 of the secure-pairing MVP). It does NOT
//! touch the network, disk, or the agent — it is the pure cryptographic
//! handshake so we can prove the design before building anything on top.
//!
//! Design (see docs/threat-model-voice-companion.md):
//! - Trust is bootstrapped **out of band** via a QR shown on the desktop. The QR
//!   carries the desktop's (responder's) static public key + a one-time secret.
//!   Because the phone learns the responder key from the QR — not from the
//!   untrusted relay — a man-in-the-middle cannot substitute keys.
//! - We use the **Noise Protocol Framework** (`snow`) — never hand-rolled crypto.
//!   Pattern `XXpsk3`: both sides exchange static keys (mutual auth) and the
//!   one-time secret is mixed in as a PSK, so the handshake fails unless the peer
//!   both presents the pinned key AND knows the secret.
//!
//! Defence in depth: after a successful handshake the initiator still verifies
//! the responder's transmitted static key equals the one pinned from the QR.
//!
//! Hito 1 is the pure crypto core: every item is exercised by the module tests
//! but has no production caller yet (the device store, approval flow, transport
//! and voice layers come in later hitos), so we silence dead-code here rather
//! than leak a wall of warnings until then.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use data_encoding::BASE64URL_NOPAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use snow::{Builder, HandshakeState, TransportState};

use crate::error::{AppError, AppResult};

/// First-contact pairing: mutual auth (XX) with the one-time pairing secret
/// mixed in as a PSK (psk3), over X25519 / ChaChaPoly / BLAKE2s.
const NOISE_PARAMS: &str = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s";

/// Reconnect of an already-paired device: both peers already know (have pinned)
/// each other's static key, so KK authenticates from the keys alone — no QR,
/// no PSK.
const NOISE_PARAMS_RECONNECT: &str = "Noise_KK_25519_ChaChaPoly_BLAKE2s";

/// A Noise static keypair. The private half is an identity secret and must never
/// leave the device (in production it lives in the OS keystore; the raw bytes
/// here are only handed to `snow`).
#[derive(Clone)]
pub struct StaticKeypair {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

fn parse_params(s: &str) -> AppResult<snow::params::NoiseParams> {
    s.parse()
        .map_err(|e| AppError::Other(format!("invalid noise params: {e}")))
}

fn params() -> AppResult<snow::params::NoiseParams> {
    parse_params(NOISE_PARAMS)
}

/// Generate a fresh static keypair using snow's CSPRNG.
pub fn generate_static() -> AppResult<StaticKeypair> {
    let kp = Builder::new(params()?)
        .generate_keypair()
        .map_err(|e| AppError::Other(format!("keypair gen: {e}")))?;
    Ok(StaticKeypair { private: kp.private, public: kp.public })
}

// ---- Desktop long-term identity ----

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    private: String,
    public: String,
}

fn identity_path() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("no data_local dir".into()))?
        .join("agent-console");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("device-identity.json"))
}

/// Load the desktop's long-term static keypair, creating it once on first run.
/// This is the identity the QR pins and that reconnects authenticate against, so
/// it must be **stable**.
///
/// NOTE (MVP): the private key is stored in a 0600 file. Production must move it
/// to the OS keystore (the `keyring` crate, already a dependency) — tracked as a
/// hardening item before this leaves the branch.
pub fn load_or_create_identity() -> AppResult<StaticKeypair> {
    let path = identity_path()?;
    if path.exists() {
        let txt = fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("read identity: {e}")))?;
        let f: IdentityFile = serde_json::from_str(&txt)
            .map_err(|e| AppError::Other(format!("parse identity: {e}")))?;
        let private = BASE64URL_NOPAD
            .decode(f.private.as_bytes())
            .map_err(|e| AppError::Other(format!("decode identity private: {e}")))?;
        let public = BASE64URL_NOPAD
            .decode(f.public.as_bytes())
            .map_err(|e| AppError::Other(format!("decode identity public: {e}")))?;
        return Ok(StaticKeypair { private, public });
    }
    let kp = generate_static()?;
    let f = IdentityFile {
        private: BASE64URL_NOPAD.encode(&kp.private),
        public: BASE64URL_NOPAD.encode(&kp.public),
    };
    let json = serde_json::to_string(&f).map_err(|e| AppError::Other(format!("serialize: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, &path)?;
    Ok(kp)
}

/// The payload encoded into the pairing QR. Authenticated only by being shown on
/// the (unlocked) desktop screen — that physical proximity is the out-of-band
/// channel. One-time, short-TTL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOffer {
    /// Rendezvous id both peers meet at on the transport (one-time).
    pub rendezvous_id: String,
    /// Responder (desktop) static public key, base64url — the phone pins this.
    pub responder_static: String,
    /// One-time pairing secret (PSK), base64url of 32 random bytes.
    pub psk: String,
    pub created_ms: i64,
    pub ttl_secs: u64,
}

impl PairingOffer {
    /// True if `now_ms` is past the offer's TTL. Time is injected (not read from
    /// the clock here) so the logic stays pure and testable.
    pub fn is_expired(&self, now_ms: i64) -> bool {
        now_ms.saturating_sub(self.created_ms) > (self.ttl_secs as i64) * 1000
    }

    /// Decode the one-time PSK from the offer (as a phone would after scanning).
    pub fn psk_bytes(&self) -> AppResult<Vec<u8>> {
        BASE64URL_NOPAD
            .decode(self.psk.as_bytes())
            .map_err(|e| AppError::Other(format!("bad psk encoding: {e}")))
    }

    /// Decode the responder (desktop) static key the phone must pin.
    pub fn responder_static_bytes(&self) -> AppResult<Vec<u8>> {
        BASE64URL_NOPAD
            .decode(self.responder_static.as_bytes())
            .map_err(|e| AppError::Other(format!("bad responder key encoding: {e}")))
    }
}

/// Build a fresh pairing offer for a desktop whose static public key is given.
/// Returns the offer (to render as a QR) plus the raw psk bytes (kept in memory
/// by the responder to drive its side of the handshake). `now_ms` injected.
pub fn new_offer(responder_static_pub: &[u8], now_ms: i64, ttl_secs: u64) -> (PairingOffer, Vec<u8>) {
    let mut rng = rand::thread_rng();
    let mut psk = vec![0u8; 32];
    rng.fill_bytes(&mut psk);
    let mut rv = vec![0u8; 16];
    rng.fill_bytes(&mut rv);
    let offer = PairingOffer {
        rendezvous_id: BASE64URL_NOPAD.encode(&rv),
        responder_static: BASE64URL_NOPAD.encode(responder_static_pub),
        psk: BASE64URL_NOPAD.encode(&psk),
        created_ms: now_ms,
        ttl_secs,
    };
    (offer, psk)
}

/// Responder (desktop) handshake state: its own static key + the offer's PSK.
pub fn build_responder(local_private: &[u8], psk: &[u8]) -> AppResult<HandshakeState> {
    Builder::new(params()?)
        .local_private_key(local_private)
        .psk(3, psk)
        .build_responder()
        .map_err(|e| AppError::Other(format!("build responder: {e}")))
}

/// Initiator (phone) handshake state: its own static key + the PSK from the QR.
pub fn build_initiator(local_private: &[u8], psk: &[u8]) -> AppResult<HandshakeState> {
    Builder::new(params()?)
        .local_private_key(local_private)
        .psk(3, psk)
        .build_initiator()
        .map_err(|e| AppError::Other(format!("build initiator: {e}")))
}

/// Reconnect handshake (KK) for an already-paired device. Both sides supply
/// their own private key AND the peer's pinned public key, so authentication
/// comes from the keys alone. Responder = desktop (knows the device's pinned
/// key, looked up in the device store); initiator = phone (knows the desktop
/// identity it pinned at pairing time).
pub fn build_responder_known(local_private: &[u8], remote_public: &[u8]) -> AppResult<HandshakeState> {
    Builder::new(parse_params(NOISE_PARAMS_RECONNECT)?)
        .local_private_key(local_private)
        .remote_public_key(remote_public)
        .build_responder()
        .map_err(|e| AppError::Other(format!("build KK responder: {e}")))
}

pub fn build_initiator_known(local_private: &[u8], remote_public: &[u8]) -> AppResult<HandshakeState> {
    Builder::new(parse_params(NOISE_PARAMS_RECONNECT)?)
        .local_private_key(local_private)
        .remote_public_key(remote_public)
        .build_initiator()
        .map_err(|e| AppError::Other(format!("build KK initiator: {e}")))
}

/// Drive a handshake to completion between two in-memory peers (initiator
/// writes first, alternating), returning the finished states. Generic over the
/// pattern (works for both XX pairing and KK reconnect). A transport carries the
/// same messages over the wire in later hitos; here it lets us prove the flow.
/// Returns an error if any step fails (e.g. a PSK or key mismatch aborts).
pub fn drive_handshake(
    mut initiator: HandshakeState,
    mut responder: HandshakeState,
) -> AppResult<(HandshakeState, HandshakeState)> {
    let mut buf = vec![0u8; 65535];
    let mut out = vec![0u8; 65535];
    let mut turn_initiator = true;
    while !(initiator.is_handshake_finished() && responder.is_handshake_finished()) {
        let (writer, reader) = if turn_initiator {
            (&mut initiator, &mut responder)
        } else {
            (&mut responder, &mut initiator)
        };
        let n = writer
            .write_message(&[], &mut buf)
            .map_err(|e| AppError::Other(format!("handshake write: {e}")))?;
        reader
            .read_message(&buf[..n], &mut out)
            .map_err(|e| AppError::Other(format!("handshake read: {e}")))?;
        turn_initiator = !turn_initiator;
    }
    Ok((initiator, responder))
}

/// After a finished handshake, confirm the peer's static key matches what we
/// pinned out-of-band (the QR). This is the explicit anti-MITM check, on top of
/// the PSK already binding the handshake.
pub fn verify_pinned_remote(hs: &HandshakeState, pinned_public: &[u8]) -> AppResult<()> {
    match hs.get_remote_static() {
        Some(remote) if remote == pinned_public => Ok(()),
        Some(_) => Err(AppError::Other(
            "remote static key does not match the pinned (QR) key — possible MITM".into(),
        )),
        None => Err(AppError::Other("no remote static key after handshake".into())),
    }
}

/// One framed message in transport mode. Thin wrapper to keep call sites tidy.
pub fn encrypt(t: &mut TransportState, plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let mut buf = vec![0u8; plaintext.len() + 16];
    let n = t
        .write_message(plaintext, &mut buf)
        .map_err(|e| AppError::Other(format!("encrypt: {e}")))?;
    buf.truncate(n);
    Ok(buf)
}

pub fn decrypt(t: &mut TransportState, ciphertext: &[u8]) -> AppResult<Vec<u8>> {
    let mut buf = vec![0u8; ciphertext.len()];
    let n = t
        .read_message(ciphertext, &mut buf)
        .map_err(|e| AppError::Other(format!("decrypt: {e}")))?;
    buf.truncate(n);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        1_700_000_000_000
    }

    #[test]
    fn pairs_and_round_trips_an_encrypted_message() {
        let desktop = generate_static().unwrap();
        let phone = generate_static().unwrap();
        let (offer, psk) = new_offer(&desktop.public, now(), 60);

        // Phone consumes the offer exactly as it would from a scanned QR.
        let pinned = offer.responder_static_bytes().unwrap();
        let offer_psk = offer.psk_bytes().unwrap();
        assert_eq!(offer_psk, psk, "psk round-trips through the offer encoding");

        let initiator = build_initiator(&phone.private, &offer_psk).unwrap();
        let responder = build_responder(&desktop.private, &psk).unwrap();
        let (init_hs, resp_hs) =
            drive_handshake(initiator, responder).expect("handshake should succeed");

        // Anti-MITM pin check: phone confirms it really spoke to the QR's desktop,
        // and the desktop now knows the phone's identity for future sessions.
        verify_pinned_remote(&init_hs, &pinned).expect("pinned key must match");
        assert_eq!(
            resp_hs.get_remote_static().unwrap(),
            phone.public.as_slice(),
            "desktop pins the phone identity (TOFU)"
        );

        // Convert to transport mode (consumes the finished handshakes).
        let mut init_t = init_hs
            .into_transport_mode()
            .expect("initiator transport");
        let mut resp_t = resp_hs
            .into_transport_mode()
            .expect("responder transport");

        // E2E message both directions.
        let ct = encrypt(&mut init_t, b"deploy status?").unwrap();
        assert_ne!(ct, b"deploy status?", "must be ciphertext on the wire");
        assert_eq!(decrypt(&mut resp_t, &ct).unwrap(), b"deploy status?");
        let ct2 = encrypt(&mut resp_t, b"tests pass, ready to commit").unwrap();
        assert_eq!(decrypt(&mut init_t, &ct2).unwrap(), b"tests pass, ready to commit");
    }

    #[test]
    fn mitm_with_wrong_static_is_detected() {
        // Attacker completes a handshake with the phone using ITS OWN static key
        // (it doesn't have the desktop's private key). The PSK is unknown to the
        // attacker too, but even if it somehow were, the pin check must catch the
        // substituted key.
        let desktop = generate_static().unwrap();
        let attacker = generate_static().unwrap();
        let phone = generate_static().unwrap();
        let (offer, psk) = new_offer(&desktop.public, now(), 60);
        let pinned = offer.responder_static_bytes().unwrap();

        let initiator = build_initiator(&phone.private, &psk).unwrap();
        // Attacker knows the PSK in this scenario but NOT the desktop key.
        let attacker_resp = build_responder(&attacker.private, &psk).unwrap();
        let (init_hs, _resp_hs) =
            drive_handshake(initiator, attacker_resp).expect("handshake completes with attacker key");

        // ...but the pin check rejects it: the remote static is the attacker's,
        // not the desktop key the phone pinned from the QR.
        assert!(
            verify_pinned_remote(&init_hs, &pinned).is_err(),
            "pin check MUST reject a substituted responder key"
        );
    }

    #[test]
    fn wrong_psk_fails_the_handshake() {
        // Without the one-time secret from the QR, the handshake itself aborts.
        let desktop = generate_static().unwrap();
        let phone = generate_static().unwrap();
        let (_offer, psk) = new_offer(&desktop.public, now(), 60);
        let mut wrong = psk.clone();
        wrong[0] ^= 0xff;

        let initiator = build_initiator(&phone.private, &wrong).unwrap();
        let responder = build_responder(&desktop.private, &psk).unwrap();
        assert!(
            drive_handshake(initiator, responder).is_err(),
            "a mismatched PSK must fail the handshake"
        );
    }

    #[test]
    fn offer_expires() {
        let desktop = generate_static().unwrap();
        let (offer, _psk) = new_offer(&desktop.public, now(), 60);
        assert!(!offer.is_expired(now() + 30_000), "fresh within TTL");
        assert!(offer.is_expired(now() + 61_000), "expired past TTL");
    }
}
