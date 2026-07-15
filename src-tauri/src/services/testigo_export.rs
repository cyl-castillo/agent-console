use std::fs;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::services::testigo_service::TestigoService;

/// The proof packet: a Testigo ledger segment wrapped in an in-toto Statement,
/// signed as a DSSE envelope, verifiable WITHOUT agent-console (standalone
/// HTML verifier / any DSSE tooling).
///
/// Honesty notes baked into the format:
/// - The embedded public key is a convenience copy; the trust anchor is the
///   keyid compared out-of-band with the publisher.
/// - Redacted events keep their original hash/prevHash fields, so chain
///   LINKAGE stays verifiable but their content hash cannot be recomputed —
///   the verifier reports them as such instead of pretending.
/// - A case export includes non-case events in range as stubs (seq/hashes
///   only): linkage verifies end-to-end, content verifies for what's shared.
const PACKET_FORMAT: &str = "testigo-proofpack/v0.1";
const PREDICATE_TYPE: &str = "https://github.com/cyl-castillo/testigo/attestation/v0.1";
const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

const KEYRING_SERVICE: &str = "agent-console-testigo";
const KEYRING_ACCOUNT: &str = "signing-key";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub path: String,
    pub verifier_path: String,
    pub event_count: usize,
    pub stub_count: usize,
    pub redaction_count: usize,
    pub key_id: String,
    pub subject_digest: String,
    pub chain_ok: bool,
}

/// Load the ed25519 signing seed from the OS keychain, generating and storing
/// one on first export. The private key never leaves the keychain entry.
fn load_or_create_seed() -> AppResult<[u8; 32]> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| AppError::Other(format!("keyring open: {e}")))?;
    match entry.get_password() {
        Ok(b64) => {
            let bytes = B64
                .decode(b64.trim())
                .map_err(|e| AppError::Other(format!("stored key decode: {e}")))?;
            bytes
                .try_into()
                .map_err(|_| AppError::Other("stored key has wrong length".into()))
        }
        Err(keyring::Error::NoEntry) => {
            let mut seed = [0u8; 32];
            getrandom::getrandom(&mut seed)
                .map_err(|e| AppError::Other(format!("entropy: {e}")))?;
            entry
                .set_password(&B64.encode(seed))
                .map_err(|e| AppError::Other(format!("keyring store: {e}")))?;
            Ok(seed)
        }
        Err(e) => Err(AppError::Other(format!("keyring read: {e}"))),
    }
}

/// keyid = sha256 hex of the raw public key — the short string publishers
/// share out-of-band so receivers can pin who signed.
fn key_id(vk: &VerifyingKey) -> String {
    let mut h = Sha256::new();
    h.update(vk.as_bytes());
    format!("{:x}", h.finalize())
}

/// DSSE v1 pre-authentication encoding: what actually gets signed.
fn pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + payload_type.len() + 32);
    out.extend_from_slice(b"DSSEv1 ");
    out.extend_from_slice(payload_type.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload_type.as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload);
    out
}

/// Conservative, quote-safe secret patterns. Applied to the raw JSONL line so
/// original bytes (and hashes) of clean events are untouched; a hit marks the
/// event redacted (its content hash becomes unverifiable, linkage stays).
/// Patterns are token-shaped on purpose — a too-eager regex that eats a JSON
/// quote would corrupt the line.
fn redact(line: &str) -> (String, usize) {
    static PATTERNS: &[(&str, &str)] = &[
        (r"AKIA[0-9A-Z]{16}", "[REDACTED:aws-key]"),
        (r"ghp_[A-Za-z0-9]{36,}", "[REDACTED:github-token]"),
        (r"github_pat_[A-Za-z0-9_]{22,}", "[REDACTED:github-token]"),
        (r"xox[baprs]-[A-Za-z0-9-]{10,}", "[REDACTED:slack-token]"),
        (r"sk-[A-Za-z0-9_-]{20,}", "[REDACTED:api-key]"),
        (
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[^-]*-----END [A-Z ]*PRIVATE KEY-----",
            "[REDACTED:private-key]",
        ),
        (r"[Bb]earer [A-Za-z0-9._~+/-]{20,}", "[REDACTED:bearer]"),
    ];
    let mut out = line.to_string();
    let mut count = 0;
    for (pat, repl) in PATTERNS {
        let re = regex::Regex::new(pat).expect("static pattern");
        let n = re.find_iter(&out).count();
        if n > 0 {
            count += n;
            out = re.replace_all(&out, *repl).into_owned();
        }
    }
    (out, count)
}

/// Build and sign a proof packet for `case_id` (or the full ledger when None)
/// into `dest_dir`, alongside a copy of the standalone verifier. Refuses to
/// export a chain that doesn't verify — a packet must never launder a broken
/// ledger into something that looks official.
pub fn export(
    testigo: &TestigoService,
    project_root: &str,
    case_id: Option<&str>,
    dest_dir: &Path,
) -> AppResult<ExportSummary> {
    let seed = load_or_create_seed()?;
    export_with_seed(testigo, project_root, case_id, dest_dir, &seed)
}

/// Keyring-free core, also the test seam (unit tests must not touch the OS
/// keychain — headless CI has none).
pub fn export_with_seed(
    testigo: &TestigoService,
    project_root: &str,
    case_id: Option<&str>,
    dest_dir: &Path,
    seed: &[u8; 32],
) -> AppResult<ExportSummary> {
    let report = testigo.verify(project_root)?;
    if !report.ok {
        return Err(AppError::Other(format!(
            "ledger chain broken at seq {:?} — refusing to export",
            report.broken_at_seq
        )));
    }

    let raw_lines = testigo.raw_lines(project_root)?;
    if raw_lines.is_empty() {
        return Err(AppError::Other("ledger is empty — nothing to export".into()));
    }

    // Select the segment: full ledger, or [first..last] event of the case
    // with non-case events reduced to linkage stubs.
    let mut events: Vec<Value> = Vec::new();
    let mut stub_count = 0usize;
    let mut redaction_count = 0usize;
    let mut included = 0usize;
    let parsed: Vec<Value> = raw_lines
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    if parsed.len() != raw_lines.len() {
        return Err(AppError::Other("ledger has unparseable lines".into()));
    }
    let in_case = |v: &Value| -> bool {
        case_id.is_none_or(|c| v.get("caseId").and_then(|x| x.as_str()) == Some(c))
    };
    let first = parsed.iter().position(in_case);
    let last = parsed.iter().rposition(in_case);
    let (Some(first), Some(last)) = (first, last) else {
        return Err(AppError::Other(format!(
            "case {case_id:?} has no events in this ledger"
        )));
    };
    let prev_hash_before = if first > 0 {
        parsed[first - 1]
            .get("hash")
            .and_then(|h| h.as_str())
            .unwrap_or("genesis")
            .to_string()
    } else {
        "genesis".to_string()
    };
    for i in first..=last {
        let v = &parsed[i];
        if in_case(v) {
            let (line, n) = redact(&raw_lines[i]);
            redaction_count += n;
            included += 1;
            events.push(json!({ "line": line, "redacted": n > 0 }));
        } else {
            stub_count += 1;
            events.push(json!({ "stub": {
                "seq": v.get("seq"),
                "prevHash": v.get("prevHash"),
                "hash": v.get("hash"),
                "kind": v.get("kind"),
            }}));
        }
    }

    // Subject digest: over the exported segment exactly as packed (post
    // redaction/stubbing) — what the receiver holds is what's signed.
    let events_body = serde_json::to_string(&events)
        .map_err(|e| AppError::Other(format!("serialize events: {e}")))?;
    let mut h = Sha256::new();
    h.update(events_body.as_bytes());
    let subject_digest = format!("{:x}", h.finalize());

    let project_name = project_root
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("project")
        .to_string();
    let exported_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let (head_seq, head_hash) = parsed
        .last()
        .map(|v| (v.get("seq").cloned(), v.get("hash").cloned()))
        .unwrap_or((None, None));

    let statement = json!({
        "_type": STATEMENT_TYPE,
        "subject": [{
            "name": case_id.unwrap_or("ledger"),
            "digest": { "sha256": subject_digest }
        }],
        "predicateType": PREDICATE_TYPE,
        "predicate": {
            "caseId": case_id,
            "project": project_name,
            "exportedAtMs": exported_at,
            "generator": format!("agent-console/{}", env!("CARGO_PKG_VERSION")),
            "range": { "fromSeq": first, "toSeq": last, "prevHashBefore": prev_hash_before },
            "ledgerHead": { "seq": head_seq, "hash": head_hash },
            "redactionCount": redaction_count,
            "events": events,
        }
    });

    let payload = serde_json::to_string(&statement)
        .map_err(|e| AppError::Other(format!("serialize statement: {e}")))?;
    let signing = SigningKey::from_bytes(seed);
    let vk = signing.verifying_key();
    let sig = signing.sign(&pae(PAYLOAD_TYPE, payload.as_bytes()));
    let kid = key_id(&vk);

    let packet = json!({
        "format": PACKET_FORMAT,
        "envelope": {
            "payloadType": PAYLOAD_TYPE,
            "payload": B64.encode(payload.as_bytes()),
            "signatures": [{ "keyid": kid, "sig": B64.encode(sig.to_bytes()) }],
        },
        "publicKey": B64.encode(vk.as_bytes()),
    });

    fs::create_dir_all(dest_dir)?;
    let stem = case_id
        .map(|c| c.replace([':', '/', '\\'], "-"))
        .unwrap_or_else(|| "ledger".into());
    let path = dest_dir.join(format!("{stem}.proofpack.json"));
    let tmp = dest_dir.join(format!("{stem}.proofpack.json.tmp"));
    fs::write(&tmp, serde_json::to_string_pretty(&packet).unwrap_or_default())?;
    fs::rename(&tmp, &path)?;

    // Ship the standalone verifier next to the packet so "send the proof"
    // is two files and zero installs for the receiver.
    let verifier_path = dest_dir.join("testigo-verifier.html");
    fs::write(&verifier_path, VERIFIER_HTML)?;

    Ok(ExportSummary {
        path: path.to_string_lossy().to_string(),
        verifier_path: verifier_path.to_string_lossy().to_string(),
        event_count: included,
        stub_count,
        redaction_count,
        key_id: kid,
        subject_digest,
        chain_ok: true,
    })
}

/// Public key info for sharing the keyid out-of-band (and showing it in UI).
pub fn public_key_info() -> AppResult<Value> {
    let seed = load_or_create_seed()?;
    let vk = SigningKey::from_bytes(&seed).verifying_key();
    Ok(json!({ "keyId": key_id(&vk), "publicKey": B64.encode(vk.as_bytes()) }))
}

const VERIFIER_HTML: &str = include_str!("../../resources/testigo-verifier.html");

/// Helper the export needs from the ledger: the raw JSONL lines, byte-exact
/// (hashes were computed over these bytes; any re-serialization would break
/// recomputation on the receiving side).
impl TestigoService {
    pub fn raw_lines(&self, project_root: &str) -> AppResult<Vec<String>> {
        let path = Self::ledger_path(project_root)?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        Ok(fs::read_to_string(&path)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Full packet round trip with an injected seed (no keychain): export a
    /// ledger with a case + interleaved events + a secret, then verify the
    /// DSSE signature, the subject digest, the stub pruning, the redaction,
    /// and the chain linkage — exactly what the HTML verifier does.
    #[test]
    fn export_packs_signs_and_verifies() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "ac-testigo-export-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = TestigoService::new();
        let root = "/proj/export";
        svc.link_case(root, 1, "t1", "jira:FIXY-9").unwrap();
        svc.on_prompt(root, 2, Some("t1"), None, Some("deploy with key ghp_0123456789abcdefghijklmnopqrstuvwxyz"), None, None)
            .unwrap();
        // An interleaved event from another terminal → becomes a stub.
        svc.on_prompt(root, 3, Some("t2"), None, Some("unrelated"), None, None)
            .unwrap();
        svc.on_turn_end(root, 4, Some("t1"), None, serde_json::json!({}))
            .unwrap();

        let seed = [7u8; 32];
        let dest = base.join("out");
        let sum =
            export_with_seed(&svc, root, Some("jira:FIXY-9"), &dest, &seed).unwrap();
        assert_eq!(sum.event_count, 3, "case_link + prompt + turn_end");
        assert_eq!(sum.stub_count, 1, "interleaved t2 prompt pruned to stub");
        assert!(sum.redaction_count >= 1, "github token redacted");
        assert!(sum.chain_ok);
        assert!(std::path::Path::new(&sum.verifier_path).exists());

        // Receiver side: parse packet, verify DSSE sig over PAE, check digest.
        let packet: Value =
            serde_json::from_str(&fs::read_to_string(&sum.path).unwrap()).unwrap();
        assert_eq!(packet["format"], PACKET_FORMAT);
        let payload = B64
            .decode(packet["envelope"]["payload"].as_str().unwrap())
            .unwrap();
        let sig_bytes = B64
            .decode(packet["envelope"]["signatures"][0]["sig"].as_str().unwrap())
            .unwrap();
        let pk_bytes: [u8; 32] = B64
            .decode(packet["publicKey"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let vk = VerifyingKey::from_bytes(&pk_bytes).unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes.try_into().unwrap());
        vk.verify(&pae(PAYLOAD_TYPE, &payload), &sig)
            .expect("DSSE signature must verify");

        let statement: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(statement["predicateType"], PREDICATE_TYPE);
        let events = statement["predicate"]["events"].as_array().unwrap();
        assert_eq!(events.len(), 4);

        // Digest covers the packed segment byte-exactly.
        let body = serde_json::to_string(&statement["predicate"]["events"]).unwrap();
        let mut h = Sha256::new();
        h.update(body.as_bytes());
        assert_eq!(
            format!("{:x}", h.finalize()),
            statement["subject"][0]["digest"]["sha256"].as_str().unwrap()
        );

        // Chain linkage: stored prevHash/hash link through clean, redacted
        // AND stub entries; clean lines also recompute.
        let mut prev = statement["predicate"]["range"]["prevHashBefore"]
            .as_str()
            .unwrap()
            .to_string();
        for e in events {
            let (prev_hash, hash, raw) = if let Some(line) = e["line"].as_str() {
                let v: Value = serde_json::from_str(line).unwrap();
                (
                    v["prevHash"].as_str().unwrap().to_string(),
                    v["hash"].as_str().unwrap().to_string(),
                    (!e["redacted"].as_bool().unwrap()).then(|| line.to_string()),
                )
            } else {
                let s = &e["stub"];
                (
                    s["prevHash"].as_str().unwrap().to_string(),
                    s["hash"].as_str().unwrap().to_string(),
                    None,
                )
            };
            assert_eq!(prev_hash, prev, "linkage must hold across all entries");
            if let Some(line) = raw {
                let idx = line.rfind("\"hash\":\"").unwrap();
                let unhashed = format!("{}\"hash\":\"\"}}", &line[..idx]);
                let mut h = Sha256::new();
                h.update(unhashed.as_bytes());
                assert_eq!(format!("{:x}", h.finalize()), hash, "clean line recomputes");
            }
            prev = hash;
        }

        // The redacted prompt no longer leaks the token.
        let prompt_entry = events
            .iter()
            .find(|e| e["line"].as_str().is_some_and(|l| l.contains("\"prompt\"")))
            .unwrap();
        assert!(prompt_entry["line"].as_str().unwrap().contains("[REDACTED:github-token]"));
        assert!(!prompt_entry["line"].as_str().unwrap().contains("ghp_0123"));
        assert_eq!(prompt_entry["redacted"], true);

        // Broken ledger → export refuses.
        let ledger = TestigoService::ledger_path(root).unwrap();
        let tampered = fs::read_to_string(&ledger).unwrap().replace("unrelated", "tampered!!");
        fs::write(&ledger, tampered).unwrap();
        let svc2 = TestigoService::new();
        assert!(export_with_seed(&svc2, root, None, &dest, &seed).is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}
