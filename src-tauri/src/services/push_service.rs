//! Web Push to a paired device (MOVIL F1b): the desktop tells the phone
//! "N decisions wait" — nothing else — so the human opens the companion
//! and decides. The whole point of the device plan is the queue nobody
//! sees; this is what makes the queue knock.
//!
//! Standards, no shortcuts: RFC 8030 (the push service POST), RFC 8291
//! (message encryption: ECDH P-256 + HKDF + AES-128-GCM, `aes128gcm`
//! content coding of RFC 8188) and RFC 8292 (VAPID: an ES256 JWT that
//! names us to the push service). The encryption is checked against RFC
//! 8291's own Appendix A vectors in the tests below.
//!
//! What a push carries: `{"type":"decisions","pending":N}`. Never a tool,
//! a command, a path or a session name — the push service (Google, Apple,
//! Mozilla) relays it and must learn nothing. The content lives on the
//! desktop; the device fetches it over the Noise channel.
//!
//! What a push is NOT: a decision channel. Nothing is approved by tapping
//! a notification; the tap opens the device app, which reconnects, reads
//! the evidence and asks.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes128Gcm, Nonce};
use data_encoding::BASE64URL_NOPAD;
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::{AppError, AppResult};

/// A browser's `PushSubscription`, as the device sends it over the channel
/// (and as it is persisted on the device record). `p256dh` and `auth` are
/// base64url (unpadded or padded; both decode).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

impl PushSubscription {
    /// The push service's host — what the UI shows instead of the endpoint
    /// (the endpoint is a capability URL; anyone holding it can push).
    pub fn host(&self) -> String {
        self.endpoint
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("?")
            .to_string()
    }

    /// Only https endpoints, only a plausible length — a subscription is
    /// stored and pushed to, so it is validated at the door.
    pub fn validate(&self) -> AppResult<()> {
        if !self.endpoint.starts_with("https://") || self.endpoint.len() > 2048 {
            return Err(AppError::InvalidArgument(
                "push endpoint must be an https URL".into(),
            ));
        }
        let key = decode_b64url(&self.p256dh)?;
        if key.len() != 65 || key[0] != 0x04 {
            return Err(AppError::InvalidArgument(
                "p256dh must be an uncompressed P-256 point (65 bytes)".into(),
            ));
        }
        if decode_b64url(&self.auth)?.len() != 16 {
            return Err(AppError::InvalidArgument("auth must be 16 bytes".into()));
        }
        Ok(())
    }
}

/// base64url with or without padding, since browsers hand out both.
pub fn decode_b64url(s: &str) -> AppResult<Vec<u8>> {
    let trimmed = s.trim().trim_end_matches('=');
    BASE64URL_NOPAD
        .decode(trimmed.as_bytes())
        .map_err(|e| AppError::InvalidArgument(format!("bad base64url: {e}")))
}

// ---- VAPID identity ----

const KEYRING_SERVICE: &str = "agent-console.remote";
const KEYRING_KEY: &str = "vapid-key";
const VAPID_FILE: &str = "vapid-key.json";

#[derive(Serialize, Deserialize)]
struct VapidFile {
    /// The 32-byte P-256 scalar, base64url.
    private: String,
}

/// The desktop's long-term VAPID keypair. Stable for as long as devices are
/// subscribed: a push service binds each subscription to the public key it
/// was created with, so a new key means every device must subscribe again.
#[derive(Clone)]
pub struct VapidKey {
    secret: SecretKey,
}

/// A fresh P-256 secret from the process CSPRNG. `from_slice` rejects the
/// (astronomically unlikely) zero or out-of-range scalar, so loop.
fn random_secret() -> SecretKey {
    loop {
        let bytes: [u8; 32] = rand::random();
        if let Ok(k) = SecretKey::from_slice(&bytes) {
            return k;
        }
    }
}

impl VapidKey {
    pub fn generate() -> Self {
        Self {
            secret: random_secret(),
        }
    }

    pub fn from_scalar(bytes: &[u8]) -> AppResult<Self> {
        Ok(Self {
            secret: SecretKey::from_slice(bytes)
                .map_err(|e| AppError::Other(format!("vapid key: {e}")))?,
        })
    }

    pub fn scalar_bytes(&self) -> Vec<u8> {
        self.secret.to_bytes().to_vec()
    }

    /// Uncompressed SEC1 point (65 bytes) — the `applicationServerKey` the
    /// browser needs to subscribe, and the `k=` of every push.
    pub fn public_bytes(&self) -> Vec<u8> {
        self.secret
            .public_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec()
    }

    pub fn public_b64(&self) -> String {
        BASE64URL_NOPAD.encode(&self.public_bytes())
    }

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn file_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join(VAPID_FILE))
    }

    fn to_json(&self) -> String {
        serde_json::to_string(&VapidFile {
            private: BASE64URL_NOPAD.encode(&self.scalar_bytes()),
        })
        .unwrap_or_default()
    }

    fn from_json(txt: &str) -> AppResult<Self> {
        let f: VapidFile = serde_json::from_str(txt)
            .map_err(|e| AppError::Other(format!("parse vapid key: {e}")))?;
        Self::from_scalar(&decode_b64url(&f.private)?)
    }

    /// Load from the 0600 file, creating it once. Tests and the keystore
    /// fallback use this directly.
    pub fn load_or_create_file() -> AppResult<Self> {
        let path = Self::file_path()?;
        if path.exists() {
            return Self::from_json(&fs::read_to_string(&path)?);
        }
        let key = Self::generate();
        write_private(&path, key.to_json().as_bytes())?;
        Ok(key)
    }

    /// Load from the OS keystore, creating it once; the file when no
    /// keystore answers. Same shape as the pairing identity's rule, simpler
    /// because losing this key costs a re-subscribe, not a re-pair.
    pub fn load_or_create() -> AppResult<Self> {
        let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_KEY) else {
            return Self::load_or_create_file();
        };
        match entry.get_password() {
            Ok(txt) => match Self::from_json(&txt) {
                Ok(k) => Ok(k),
                Err(_) => {
                    let k = Self::generate();
                    let _ = entry.set_password(&k.to_json());
                    Ok(k)
                }
            },
            Err(keyring::Error::NoEntry) => {
                let k = Self::load_or_create_file()?;
                if entry.set_password(&k.to_json()).is_ok() {
                    if let Ok(p) = Self::file_path() {
                        let _ = fs::remove_file(p);
                    }
                }
                Ok(k)
            }
            Err(_) => Self::load_or_create_file(),
        }
    }

    /// RFC 8292 §2: `vapid t=<JWT>, k=<public key>`. The JWT's audience is
    /// the push service's origin, it expires in `ttl_secs` (≤ 24 h), and
    /// `sub` says who to contact about the traffic.
    pub fn authorization(&self, endpoint: &str, now_secs: u64, ttl_secs: u64) -> AppResult<String> {
        let aud = origin_of(endpoint)?;
        let header = BASE64URL_NOPAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({
            "aud": aud,
            "exp": now_secs + ttl_secs.min(24 * 3600),
            "sub": "https://agent-console.dev",
        });
        let claims = BASE64URL_NOPAD.encode(claims.to_string().as_bytes());
        let signing_input = format!("{header}.{claims}");
        let signer = SigningKey::from(&self.secret);
        let sig: Signature = signer.sign(signing_input.as_bytes());
        let jwt = format!(
            "{signing_input}.{}",
            BASE64URL_NOPAD.encode(&sig.to_bytes())
        );
        Ok(format!("vapid t={jwt}, k={}", self.public_b64()))
    }
}

/// `https://host[:port]` of an endpoint URL.
pub fn origin_of(url: &str) -> AppResult<String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| AppError::InvalidArgument("endpoint has no scheme".into()))?;
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() {
        return Err(AppError::InvalidArgument("endpoint has no host".into()));
    }
    Ok(format!("{scheme}://{host}"))
}

#[cfg(unix)]
fn write_private(path: &PathBuf, bytes: &[u8]) -> AppResult<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &PathBuf, bytes: &[u8]) -> AppResult<()> {
    fs::write(path, bytes)?;
    Ok(())
}

// ---- RFC 8291 encryption ----

/// Record size we declare; one record holds everything we ever send.
const RECORD_SIZE: u32 = 4096;
/// Largest plaintext a single-record message may carry (RFC 8188: rs minus
/// the tag and the delimiter).
const MAX_PLAINTEXT: usize = RECORD_SIZE as usize - 16 - 1;

/// Encrypt `plaintext` for the subscription with a fresh ephemeral key and
/// salt. The output is the whole request body (`aes128gcm` header included).
pub fn encrypt(sub: &PushSubscription, plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let ephemeral = random_secret();
    let salt: [u8; 16] = rand::random();
    encrypt_with(&ephemeral, &salt, sub, plaintext)
}

/// The deterministic core, so the RFC's vectors can drive it.
pub fn encrypt_with(
    as_secret: &SecretKey,
    salt: &[u8; 16],
    sub: &PushSubscription,
    plaintext: &[u8],
) -> AppResult<Vec<u8>> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(AppError::InvalidArgument("push payload too large".into()));
    }
    let ua_public_bytes = decode_b64url(&sub.p256dh)?;
    let ua_public = PublicKey::from_sec1_bytes(&ua_public_bytes)
        .map_err(|e| AppError::InvalidArgument(format!("p256dh: {e}")))?;
    let auth = decode_b64url(&sub.auth)?;
    let as_public_bytes = as_secret
        .public_key()
        .to_sec1_point(false)
        .as_bytes()
        .to_vec();

    // ecdh_secret, then the two HKDF stages of RFC 8291 §3.3–3.4.
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_public.as_affine());
    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(&ua_public_bytes);
    key_info.extend_from_slice(&as_public_bytes);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|_| AppError::Other("hkdf ikm".into()))?;
    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|_| AppError::Other("hkdf cek".into()))?;
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|_| AppError::Other("hkdf nonce".into()))?;

    // One record: plaintext || 0x02 (the last-record delimiter), sealed.
    let mut record = Vec::with_capacity(plaintext.len() + 1);
    record.extend_from_slice(plaintext);
    record.push(0x02);
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|_| AppError::Other("aes key".into()))?;
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &record,
                aad: &[],
            },
        )
        .map_err(|_| AppError::Other("aes-gcm seal".into()))?;

    // RFC 8188 header: salt | rs | idlen | keyid (= our public key).
    let mut body = Vec::with_capacity(16 + 4 + 1 + 65 + sealed.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(as_public_bytes.len() as u8);
    body.extend_from_slice(&as_public_bytes);
    body.extend_from_slice(&sealed);
    Ok(body)
}

// ---- Sending ----

/// What a delivery attempt came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// 201 (or any 2xx): accepted by the push service.
    Sent,
    /// 404 / 410: the subscription is dead; drop it.
    Gone,
    /// Anything else, with the status and the service's first line.
    Failed(String),
}

/// How long the push service keeps an undelivered message. Short: a "N
/// decisions wait" from an hour ago is noise, and the count is fetched
/// fresh when the app opens anyway.
pub const PUSH_TTL_SECS: u64 = 120;

/// POST one encrypted message (RFC 8030 §5 + 8291 + 8292). Blocking —
/// callers run it off the UI thread.
pub fn send(vapid: &VapidKey, sub: &PushSubscription, payload: &[u8]) -> AppResult<Delivery> {
    sub.validate()?;
    let body = encrypt(sub, payload)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let auth = vapid.authorization(&sub.endpoint, now, 12 * 3600)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Other(format!("push client: {e}")))?;
    let resp = client
        .post(&sub.endpoint)
        .header("TTL", PUSH_TTL_SECS.to_string())
        .header("Urgency", "high")
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("Authorization", auth)
        .body(body)
        .send()
        .map_err(|e| AppError::Other(format!("push: {e}")))?;
    let status = resp.status();
    Ok(if status.is_success() {
        Delivery::Sent
    } else if status.as_u16() == 404 || status.as_u16() == 410 {
        Delivery::Gone
    } else {
        let text = resp.text().unwrap_or_default();
        Delivery::Failed(format!(
            "{} {}",
            status.as_u16(),
            text.lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(200)
                .collect::<String>()
        ))
    })
}

/// The one payload the desktop ever pushes.
pub fn decisions_payload(pending: usize) -> Vec<u8> {
    serde_json::json!({ "type": "decisions", "pending": pending })
        .to_string()
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::VerifyingKey;

    fn b64(s: &str) -> Vec<u8> {
        decode_b64url(s).unwrap()
    }

    /// RFC 8291 Appendix A, byte for byte: the application server's key
    /// and salt are the RFC's, so the whole body must equal the RFC's.
    #[test]
    fn rfc_8291_appendix_a_vector_matches() {
        let as_secret =
            SecretKey::from_slice(&b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&b64("DGv6ra1nlYgDCS1FRnbzlw"));
        let sub = PushSubscription {
            endpoint: "https://push.example.net/push/JzLQ3raZJfFBR0aqvOMsLrt54w4rJUsV".into(),
            p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".into(),
            auth: "BTBZMqHH6r4Tts7J_aSIgg".into(),
        };
        let plaintext = b"When I grow up, I want to be a watermelon";
        let body = encrypt_with(&as_secret, &salt, &sub, plaintext).unwrap();
        assert_eq!(
            BASE64URL_NOPAD.encode(&body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
        // Header layout: salt, rs 4096, idlen 65, our public key.
        assert_eq!(&body[..16], &salt);
        assert_eq!(&body[16..20], &4096u32.to_be_bytes());
        assert_eq!(body[20], 65);
        assert_eq!(
            BASE64URL_NOPAD.encode(&body[21..86]),
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"
        );
        // A fresh key and salt each time: the same text never encrypts the same.
        let a = encrypt(&sub, plaintext).unwrap();
        let b = encrypt(&sub, plaintext).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), body.len());
    }

    /// VAPID: the header has the RFC 8292 shape, the JWT verifies under our
    /// public key, and its claims name the push service's origin, a bounded
    /// expiry and a contact.
    #[test]
    fn vapid_authorization_is_an_es256_jwt_for_the_endpoints_origin() {
        let key = VapidKey::generate();
        let auth = key
            .authorization(
                "https://fcm.googleapis.com/fcm/send/abc:def",
                1_700_000_000,
                99 * 3600,
            )
            .unwrap();
        let (t, k) = auth
            .strip_prefix("vapid t=")
            .and_then(|s| s.split_once(", k="))
            .expect("vapid t=…, k=…");
        assert_eq!(k, key.public_b64());
        assert_eq!(b64(k).len(), 65);
        let mut parts = t.split('.');
        let (h, c, s) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        assert_eq!(b64(h), br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims: serde_json::Value = serde_json::from_slice(&b64(c)).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["exp"], 1_700_000_000 + 24 * 3600, "capped at 24 h");
        assert_eq!(claims["sub"], "https://agent-console.dev");
        let sig = Signature::from_slice(&b64(s)).unwrap();
        let verifier = VerifyingKey::from_sec1_bytes(&b64(k)).unwrap();
        verifier
            .verify(format!("{h}.{c}").as_bytes(), &sig)
            .expect("signature verifies under the advertised key");
        // The key round-trips through its scalar (what the keystore holds).
        let again = VapidKey::from_scalar(&key.scalar_bytes()).unwrap();
        assert_eq!(again.public_b64(), key.public_b64());
    }

    #[test]
    fn subscriptions_are_validated_at_the_door_and_show_only_their_host() {
        let good = PushSubscription {
            endpoint: "https://updates.push.services.mozilla.com/wpush/v2/gAAAA".into(),
            p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".into(),
            auth: "BTBZMqHH6r4Tts7J_aSIgg==".into(),
        };
        good.validate().unwrap();
        assert_eq!(good.host(), "updates.push.services.mozilla.com");
        let http = PushSubscription {
            endpoint: "http://evil/".into(),
            ..good.clone()
        };
        assert!(http.validate().is_err());
        let short = PushSubscription {
            auth: "AAAA".into(),
            ..good.clone()
        };
        assert!(short.validate().is_err());
        let not_a_point = PushSubscription {
            p256dh: "AAAA".into(),
            ..good
        };
        assert!(not_a_point.validate().is_err());
        assert_eq!(
            String::from_utf8(decisions_payload(3)).unwrap(),
            r#"{"pending":3,"type":"decisions"}"#
        );
    }
}
