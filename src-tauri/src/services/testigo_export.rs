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
use crate::services::testigo_service::{ProofEvent, TestigoService};

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
    /// TSA that timestamped the packet signature (RFC 3161, spec §2.5), when
    /// the project opted in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_tsa: Option<String>,
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
        // Home directories: every prompt records its cwd, and a packet that
        // leaves the machine should not name the operator's account.
        (r"/home/[A-Za-z0-9._-]+", "[REDACTED:home]"),
        (r"/Users/[A-Za-z0-9._-]+", "[REDACTED:home]"),
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
    manual_redact: &[u64],
) -> AppResult<ExportSummary> {
    let seed = load_or_create_seed()?;
    let tsa = testigo.settings(project_root).timestamp_tsa;
    export_with_seed(
        testigo,
        project_root,
        case_id,
        dest_dir,
        &seed,
        manual_redact,
        tsa.as_deref(),
    )
}

/// One reviewable entry of a pending export: what WOULD be packed, shown to
/// the human before anything is signed. `line` is the post-auto-redaction
/// content; stubs carry no content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewEntry {
    pub seq: u64,
    pub ts: i64,
    pub kind: String,
    pub actor: String,
    pub stub: bool,
    /// Auto (token-pattern) redaction already hit this event.
    pub auto_redacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_id: Option<String>,
    pub entries: Vec<PreviewEntry>,
}

/// The pre-sign review: exactly the entries `export` would pack (selection +
/// auto redaction applied), so the human can mark additional events for
/// manual redaction before the packet is signed. Nothing is written.
pub fn preview(
    testigo: &TestigoService,
    project_root: &str,
    case_id: Option<&str>,
) -> AppResult<ExportPreview> {
    let seg = select_segment(testigo, project_root, case_id)?;
    let mut entries = Vec::new();
    for i in seg.first..=seg.last {
        let v = &seg.parsed[i];
        let seq = v.get("seq").and_then(|s| s.as_u64()).unwrap_or(i as u64);
        let ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
        let kind = v
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string();
        let actor = v
            .get("actor")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        if seg.in_case(v) {
            let (line, n) = redact(&seg.raw_lines[i]);
            entries.push(PreviewEntry {
                seq,
                ts,
                kind,
                actor,
                stub: false,
                auto_redacted: n > 0,
                line: Some(line),
            });
        } else {
            entries.push(PreviewEntry {
                seq,
                ts,
                kind,
                actor,
                stub: true,
                auto_redacted: false,
                line: None,
            });
        }
    }
    Ok(ExportPreview {
        case_id: case_id.map(String::from),
        entries,
    })
}

/// Segment selection shared by preview and export: full ledger, or
/// [first..last] event of the case (with the rest becoming stubs downstream).
struct Segment {
    raw_lines: Vec<String>,
    parsed: Vec<Value>,
    first: usize,
    last: usize,
    prev_hash_before: String,
    case_id: Option<String>,
}

impl Segment {
    fn in_case(&self, v: &Value) -> bool {
        self.case_id
            .as_deref()
            .is_none_or(|c| v.get("caseId").and_then(|x| x.as_str()) == Some(c))
    }
}

fn select_segment(
    testigo: &TestigoService,
    project_root: &str,
    case_id: Option<&str>,
) -> AppResult<Segment> {
    let report = testigo.verify(project_root)?;
    if !report.ok {
        return Err(AppError::Other(format!(
            "ledger chain broken at seq {:?} — refusing to export",
            report.broken_at_seq
        )));
    }
    let raw_lines = testigo.raw_lines(project_root)?;
    if raw_lines.is_empty() {
        return Err(AppError::Other(
            "ledger is empty — nothing to export".into(),
        ));
    }
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
    Ok(Segment {
        raw_lines,
        parsed,
        first,
        last,
        prev_hash_before,
        case_id: case_id.map(String::from),
    })
}

/// Manually redact one event line per the protocol: the payload is replaced
/// with a marker, every other field — crucially `prevHash`/`hash` — stays
/// intact, so chain linkage remains verifiable while the content is gone.
fn redact_manually(line: &str) -> AppResult<String> {
    let mut ev: ProofEvent = serde_json::from_str(line)
        .map_err(|e| AppError::Other(format!("parse event for manual redaction: {e}")))?;
    ev.payload = json!({ "redacted": "manual" });
    serde_json::to_string(&ev).map_err(|e| AppError::Other(format!("serialize redacted: {e}")))
}

/// Keyring-free core, also the test seam (unit tests must not touch the OS
/// keychain — headless CI has none). `manual_redact` lists the seqs the human
/// marked in the pre-sign review; `tsa` requests an RFC 3161 timestamp of the
/// signature from that URL (None keeps tests and default exports offline).
pub fn export_with_seed(
    testigo: &TestigoService,
    project_root: &str,
    case_id: Option<&str>,
    dest_dir: &Path,
    seed: &[u8; 32],
    manual_redact: &[u64],
    tsa: Option<&str>,
) -> AppResult<ExportSummary> {
    let seg = select_segment(testigo, project_root, case_id)?;
    let mut events: Vec<Value> = Vec::new();
    let mut stub_count = 0usize;
    let mut redaction_count = 0usize;
    let mut included = 0usize;
    for i in seg.first..=seg.last {
        let v = &seg.parsed[i];
        if seg.in_case(v) {
            let (mut line, n) = redact(&seg.raw_lines[i]);
            let mut redacted = n > 0;
            let seq = v.get("seq").and_then(|s| s.as_u64());
            if seq.is_some_and(|s| manual_redact.contains(&s)) {
                line = redact_manually(&line)?;
                redacted = true;
            }
            // Spec §2.3 (normative since the corpus cross of 2026-07-21):
            // redactionCount counts ENTRIES carrying redacted:true, not
            // pattern hits — a line with three secrets is one redaction, and
            // an auto-redacted line that is also marked manually is still one.
            // Counting hits is what left the published Fixy packet (v0.48.1)
            // declaring 12 over 8 redacted entries.
            if redacted {
                redaction_count += 1;
            }
            included += 1;
            events.push(json!({ "line": line, "redacted": redacted }));
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
    let (first, last) = (seg.first, seg.last);
    let (parsed, prev_hash_before) = (seg.parsed, seg.prev_hash_before);

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

    // Process context (spec §2.6), derived from what the packet carries.
    let (provider, context_artifacts, window) = process_context(&parsed, &events);
    let mut predicate = json!({
        "caseId": case_id,
        "project": project_name,
        "exportedAtMs": exported_at,
        "generator": format!("agent-console/{}", env!("CARGO_PKG_VERSION")),
        "range": { "fromSeq": first, "toSeq": last, "prevHashBefore": prev_hash_before },
        "ledgerHead": { "seq": head_seq, "hash": head_hash },
        "redactionCount": redaction_count,
    });
    predicate["provider"] = provider;
    if !context_artifacts.is_empty() {
        predicate["contextArtifacts"] = Value::Array(context_artifacts);
    }
    if let Some((start, end)) = window {
        predicate["startTimestamp"] = json!(rfc3339_ms(start));
        predicate["endTimestamp"] = json!(rfc3339_ms(end));
    }
    predicate["events"] = Value::Array(events);
    let statement = json!({
        "_type": STATEMENT_TYPE,
        "subject": [{
            "name": case_id.unwrap_or("ledger"),
            "digest": { "sha256": subject_digest }
        }],
        "predicateType": PREDICATE_TYPE,
        "predicate": predicate,
    });

    let payload = serde_json::to_string(&statement)
        .map_err(|e| AppError::Other(format!("serialize statement: {e}")))?;
    let signing = SigningKey::from_bytes(seed);
    let vk = signing.verifying_key();
    let sig = signing.sign(&pae(PAYLOAD_TYPE, payload.as_bytes()));
    let kid = key_id(&vk);

    let mut packet = json!({
        "format": PACKET_FORMAT,
        "envelope": {
            "payloadType": PAYLOAD_TYPE,
            "payload": B64.encode(payload.as_bytes()),
            "signatures": [{ "keyid": kid, "sig": B64.encode(sig.to_bytes()) }],
        },
        "publicKey": B64.encode(vk.as_bytes()),
    });
    // Opt-in trusted timestamp over the signature (spec §2.5). The user asked
    // for it, so a TSA failure fails the export instead of silently producing
    // a packet that misses what they opted into.
    if let Some(url) = tsa {
        packet["timestamp"] = crate::services::testigo_timestamp::obtain(url, &sig.to_bytes())?;
    }

    fs::create_dir_all(dest_dir)?;
    let stem = case_id
        .map(|c| c.replace([':', '/', '\\'], "-"))
        .unwrap_or_else(|| "ledger".into());
    let path = dest_dir.join(format!("{stem}.proofpack.json"));
    let tmp = dest_dir.join(format!("{stem}.proofpack.json.tmp"));
    fs::write(
        &tmp,
        serde_json::to_string_pretty(&packet).unwrap_or_default(),
    )?;
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
        timestamp_tsa: tsa.map(String::from),
    })
}

/// Process context (spec §2.6): `(provider, contextArtifacts, (startTs, endTs))`,
/// DERIVED from evidence, never typed in at export. Models are session
/// properties, so `session_start` / `model_switch` events of the sessions
/// involved count even when they sit outside the exported range; instruction
/// digests come only from the packed `prompt` lines — a redacted prompt
/// contributes nothing, because the packet no longer shows it. The window is
/// the `ts` of the first and last non-stub packed entries, which is exactly
/// what verifiers recompute.
fn process_context(ledger: &[Value], packed: &[Value]) -> (Value, Vec<Value>, Option<(i64, i64)>) {
    let lines: Vec<Value> = packed
        .iter()
        .filter_map(|e| e.get("line").and_then(Value::as_str))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    let ts = |v: &Value| v.get("ts").and_then(Value::as_i64);
    let window = match (lines.first().and_then(ts), lines.last().and_then(ts)) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    };

    let sessions: std::collections::HashSet<&str> = lines
        .iter()
        .filter_map(|v| v.get("sessionId").and_then(Value::as_str))
        .collect();
    let mut models: Vec<String> = Vec::new();
    for v in ledger {
        let in_session = v
            .get("sessionId")
            .and_then(Value::as_str)
            .is_some_and(|s| sessions.contains(s));
        if !in_session {
            continue;
        }
        let model = match v.get("kind").and_then(Value::as_str) {
            Some("session_start") => v.pointer("/payload/model"),
            Some("model_switch") => v.pointer("/payload/to"),
            _ => None,
        };
        if let Some(m) = model.and_then(Value::as_str) {
            if !m.is_empty() && !models.iter().any(|x| x == m) {
                models.push(m.to_string());
            }
        }
    }
    let mut provider = json!({
        "harness": { "name": "agent-console", "version": env!("CARGO_PKG_VERSION") }
    });
    if !models.is_empty() {
        provider["languageModels"] =
            Value::Array(models.iter().map(|m| json!({ "resolved": m })).collect());
    }

    let is_hex64 =
        |s: &str| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    let mut context: Vec<Value> = Vec::new();
    for v in &lines {
        if v.get("kind").and_then(Value::as_str) != Some("prompt") {
            continue;
        }
        let Some(arr) = v.pointer("/payload/context").and_then(Value::as_array) else {
            continue;
        };
        for c in arr {
            let (Some(uri), Some(sha)) = (
                c.get("uri").and_then(Value::as_str),
                c.get("sha256").and_then(Value::as_str),
            ) else {
                continue;
            };
            if uri.is_empty() || !is_hex64(sha) {
                continue;
            }
            if context
                .iter()
                .any(|x| x["uri"] == uri && x["digest"]["sha256"] == sha)
            {
                continue;
            }
            context
                .push(json!({ "tags": ["instructions"], "uri": uri, "digest": { "sha256": sha } }));
        }
    }
    (provider, context, window)
}

/// Epoch milliseconds → RFC 3339 with millisecond precision and a `Z`
/// designator (what `Date#toISOString` produces), so both producers emit
/// the same bytes for the same instant. Civil-from-days per Howard Hinnant.
fn rfc3339_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
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

    #[test]
    fn home_directories_are_redacted() {
        let (line, n) = redact(r#"{"cwd":"/home/carlos/Documents/x","p":"/Users/ada/y","k":"v"}"#);
        assert_eq!(n, 2);
        assert!(line.contains("[REDACTED:home]/Documents/x") && line.contains("[REDACTED:home]/y"));
        assert!(!line.contains("carlos") && !line.contains("ada"));
        assert_eq!(redact("/tmp/proj"), ("/tmp/proj".to_string(), 0));
    }

    #[test]
    fn rfc3339_ms_matches_date_to_iso_string() {
        assert_eq!(rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_ms(1784133679508), "2026-07-15T16:41:19.508Z");
        assert_eq!(rfc3339_ms(951782400000), "2000-02-29T00:00:00.000Z");
        assert_eq!(rfc3339_ms(-1), "1969-12-31T23:59:59.999Z");
    }

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
        // A cwd with an instruction file: its digest must land in the prompt
        // event (spec §1.7) and surface as a context artifact (spec §2.6).
        let cwd = base.join("cwd");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(cwd.join("CLAUDE.md"), b"never push to main\n").unwrap();
        svc.link_case(root, 1, "t1", "jira:FIXY-9").unwrap();
        svc.on_prompt(
            root,
            2,
            Some("t1"),
            None,
            Some("deploy with key ghp_0123456789abcdefghijklmnopqrstuvwxyz"),
            None,
            Some(cwd.to_str().unwrap()),
        )
        .unwrap();
        // An interleaved event from another terminal → becomes a stub.
        svc.on_prompt(root, 3, Some("t2"), None, Some("unrelated"), None, None)
            .unwrap();
        svc.on_turn_end(root, 4, Some("t1"), None, serde_json::json!({}))
            .unwrap();

        // The pre-sign review lists exactly what would be packed.
        let pv = preview(&svc, root, Some("jira:FIXY-9")).unwrap();
        assert_eq!(pv.entries.len(), 4);
        assert!(
            pv.entries[1].auto_redacted,
            "token prompt flagged in preview"
        );
        assert!(pv.entries[2].stub && pv.entries[2].line.is_none());

        let seed = [7u8; 32];
        let dest = base.join("out");
        let sum =
            export_with_seed(&svc, root, Some("jira:FIXY-9"), &dest, &seed, &[], None).unwrap();
        assert_eq!(sum.event_count, 3, "case_link + prompt + turn_end");
        assert_eq!(sum.stub_count, 1, "interleaved t2 prompt pruned to stub");
        assert!(sum.redaction_count >= 1, "github token redacted");
        assert!(sum.chain_ok);
        assert!(std::path::Path::new(&sum.verifier_path).exists());

        // Receiver side: parse packet, verify DSSE sig over PAE, check digest.
        let packet: Value = serde_json::from_str(&fs::read_to_string(&sum.path).unwrap()).unwrap();
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

        // §2.3: redactionCount is the number of redacted ENTRIES, not hits.
        let pred = &statement["predicate"];
        let redacted_entries = events.iter().filter(|e| e["redacted"] == true).count();
        assert_eq!(
            pred["redactionCount"],
            json!(redacted_entries),
            "redactionCount counts entries carrying redacted:true (spec §2.3)"
        );

        // §2.6 process context, derived from the packed lines.
        assert_eq!(pred["provider"]["harness"]["name"], "agent-console");
        assert_eq!(
            pred["provider"]["harness"]["version"],
            env!("CARGO_PKG_VERSION")
        );
        assert!(
            pred["provider"].get("languageModels").is_none(),
            "no session_start in this ledger ⇒ no model claimed"
        );
        assert_eq!(
            pred["startTimestamp"], "1970-01-01T00:00:00.001Z",
            "window start = first packed line (case_link, ts 1)"
        );
        assert_eq!(
            pred["endTimestamp"], "1970-01-01T00:00:00.004Z",
            "window end = last packed line (turn_end, ts 4; the t2 stub at ts 3 does not count)"
        );
        let mut h = Sha256::new();
        h.update(b"never push to main\n");
        let want = format!("{:x}", h.finalize());
        assert_eq!(
            pred["contextArtifacts"],
            json!([{ "tags": ["instructions"], "uri": "CLAUDE.md", "digest": { "sha256": want } }]),
            "instruction digest hashed at prompt time, derived at export"
        );

        // Digest covers the packed segment byte-exactly.
        let body = serde_json::to_string(&statement["predicate"]["events"]).unwrap();
        let mut h = Sha256::new();
        h.update(body.as_bytes());
        assert_eq!(
            format!("{:x}", h.finalize()),
            statement["subject"][0]["digest"]["sha256"]
                .as_str()
                .unwrap()
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
                // Byte-exact, final-member recompute (spec §1.5) — never
                // rebuild the suffix from the last `"hash":"` occurrence.
                assert_eq!(
                    crate::services::testigo_service::recompute_line_hash(&line).as_deref(),
                    Some(hash.as_str()),
                    "clean line recomputes"
                );
            }
            prev = hash;
        }

        // The redacted prompt no longer leaks the token.
        let prompt_entry = events
            .iter()
            .find(|e| e["line"].as_str().is_some_and(|l| l.contains("\"prompt\"")))
            .unwrap();
        assert!(prompt_entry["line"]
            .as_str()
            .unwrap()
            .contains("[REDACTED:github-token]"));
        assert!(!prompt_entry["line"].as_str().unwrap().contains("ghp_0123"));
        assert_eq!(prompt_entry["redacted"], true);

        // Manual redaction (pre-sign review): payload replaced, hashes and
        // linkage intact, redacted flagged, count incremented.
        let sum2 =
            export_with_seed(&svc, root, Some("jira:FIXY-9"), &dest, &seed, &[3], None).unwrap();
        assert!(sum2.redaction_count >= 2, "auto + manual");
        let packet2: Value =
            serde_json::from_str(&fs::read_to_string(&sum2.path).unwrap()).unwrap();
        let payload2 = B64
            .decode(packet2["envelope"]["payload"].as_str().unwrap())
            .unwrap();
        let st2: Value = serde_json::from_slice(&payload2).unwrap();
        let evs2 = st2["predicate"]["events"].as_array().unwrap();
        let turn_end = evs2
            .iter()
            .find(|e| {
                e["line"]
                    .as_str()
                    .is_some_and(|l| l.contains("\"turn_end\""))
            })
            .unwrap();
        assert_eq!(turn_end["redacted"], true);
        let v2: Value = serde_json::from_str(turn_end["line"].as_str().unwrap()).unwrap();
        assert_eq!(v2["payload"]["redacted"], "manual", "payload replaced");
        // Linkage still holds through the manually redacted event.
        let mut prev2 = st2["predicate"]["range"]["prevHashBefore"]
            .as_str()
            .unwrap()
            .to_string();
        for e in evs2 {
            let (ph, h) = if let Some(line) = e["line"].as_str() {
                let v: Value = serde_json::from_str(line).unwrap();
                (
                    v["prevHash"].as_str().unwrap().to_string(),
                    v["hash"].as_str().unwrap().to_string(),
                )
            } else {
                let s = &e["stub"];
                (
                    s["prevHash"].as_str().unwrap().to_string(),
                    s["hash"].as_str().unwrap().to_string(),
                )
            };
            assert_eq!(ph, prev2);
            prev2 = h;
        }

        // Timestamp opt-in with an unreachable TSA → the export FAILS instead
        // of silently writing a packet without what the user asked for.
        let bad_tsa = export_with_seed(
            &svc,
            root,
            Some("jira:FIXY-9"),
            &dest,
            &seed,
            &[],
            Some("http://127.0.0.1:9/tsr"),
        );
        assert!(bad_tsa.is_err(), "unreachable TSA must fail the export");

        // Broken ledger → export refuses (preview too).
        let ledger = TestigoService::ledger_path(root).unwrap();
        let tampered = fs::read_to_string(&ledger)
            .unwrap()
            .replace("unrelated", "tampered!!");
        fs::write(&ledger, tampered).unwrap();
        let svc2 = TestigoService::new();
        assert!(export_with_seed(&svc2, root, None, &dest, &seed, &[], None).is_err());
        assert!(preview(&svc2, root, None).is_err());

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Re-export a real case from this machine's ledger with the keychain
    /// signing key — the operator's tool for regenerating a published packet
    /// (e.g. the Fixy case study after a format fix). #[ignore] because it
    /// reads the OS keychain and the user's real ledger; run on demand:
    ///
    ///   TESTIGO_REGEN_ROOT=/path/to/project TESTIGO_REGEN_CASE=term:… \
    ///   TESTIGO_REGEN_REDACT=2,4,5 TESTIGO_REGEN_OUT=/tmp/out \
    ///   cargo test --lib regenerate_case_packet -- --ignored --nocapture
    ///
    /// Writes two packets: `<out>/private/` (no manual redaction — keep it
    /// private) and `<out>/` (manually redacted seqs — the one to publish),
    /// and prints both subject digests and the key id for the README table.
    #[test]
    #[ignore]
    fn regenerate_case_packet_from_env() {
        let _env = crate::test_support::lock_env();
        let root = std::env::var("TESTIGO_REGEN_ROOT").expect("TESTIGO_REGEN_ROOT");
        let case = std::env::var("TESTIGO_REGEN_CASE").ok();
        let out = std::path::PathBuf::from(
            std::env::var("TESTIGO_REGEN_OUT").expect("TESTIGO_REGEN_OUT"),
        );
        let redact: Vec<u64> = std::env::var("TESTIGO_REGEN_REDACT")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().parse().expect("seq number"))
            .collect();
        let svc = TestigoService::new();
        let full = export(&svc, &root, case.as_deref(), &out.join("private"), &[]).unwrap();
        let redacted = export(&svc, &root, case.as_deref(), &out, &redact).unwrap();
        assert_eq!(full.key_id, redacted.key_id);
        assert_eq!(
            redacted.redaction_count,
            full.redaction_count + redact.len()
        );
        println!("key id:                      {}", redacted.key_id);
        println!(
            "digest (private, full):      {}  -> {}",
            full.subject_digest, full.path
        );
        println!(
            "digest (published, redacted): {}  -> {}",
            redacted.subject_digest, redacted.path
        );
        println!(
            "events {} · stubs {} · redacted entries {}{}",
            redacted.event_count,
            redacted.stub_count,
            redacted.redaction_count,
            redacted
                .timestamp_tsa
                .as_deref()
                .map(|t| format!(" · RFC 3161 via {t}"))
                .unwrap_or_default()
        );
    }

    /// End-to-end against the real freetsa.org TSA — network, so #[ignore]:
    /// run on demand with `cargo test export_timestamps -- --ignored`.
    /// Exports with the timestamp opt-in and checks the packet carries a
    /// granted token whose imprint is sha256 of this packet's signature.
    #[test]
    #[ignore]
    fn export_timestamps_with_real_tsa() {
        use crate::services::testigo_timestamp;
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-testigo-tsa-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = TestigoService::new();
        let root = "/proj/tsa";
        svc.on_prompt(root, 1, Some("t1"), None, Some("timestamp me"), None, None)
            .unwrap();

        let seed = [9u8; 32];
        let dest = base.join("out");
        let sum = export_with_seed(
            &svc,
            root,
            None,
            &dest,
            &seed,
            &[],
            Some("https://freetsa.org/tsr"),
        )
        .unwrap();
        assert_eq!(
            sum.timestamp_tsa.as_deref(),
            Some("https://freetsa.org/tsr")
        );

        let packet: Value = serde_json::from_str(&fs::read_to_string(&sum.path).unwrap()).unwrap();
        let tsp = &packet["timestamp"];
        assert_eq!(tsp["type"], "rfc3161");
        assert_eq!(tsp["hashAlg"], "sha256");
        let sig = B64
            .decode(packet["envelope"]["signatures"][0]["sig"].as_str().unwrap())
            .unwrap();
        let mut h = Sha256::new();
        h.update(&sig);
        let digest: [u8; 32] = h.finalize().into();
        let imprint: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(tsp["messageImprint"].as_str().unwrap(), imprint);
        let token = B64.decode(tsp["token"].as_str().unwrap()).unwrap();
        testigo_timestamp::check_response(&token, &digest).expect("granted token");

        let _ = std::fs::remove_dir_all(&base);
    }
}
