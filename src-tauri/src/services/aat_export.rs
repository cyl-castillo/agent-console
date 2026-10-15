//! D3: export the Testigo ledger as an IETF Agent Audit Trail
//! (draft-sharif-agent-audit-trail-08): one JSON Lines file per session,
//! hash-chained with SHA-256 over RFC 8785 canonical JSON, genesis and close
//! lifecycle records, gap-free sequence numbers — so a third party verifies
//! a proof-on-PR offline with any conforming verifier (e.g. `last-record`),
//! never with Agent Console. The Testigo seq and hash travel inside
//! `action_detail`, so the two chains cross-reference each other.
//!
//! Trust level is L1 throughout: the agent is identified (session, caller
//! name) and the console records independently of it over a token-gated
//! loopback, but nothing is cryptographically attested. Honest, not inflated.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::services::testigo_service::{ProofEvent, TestigoService};

pub const AAT_DRAFT: &str = "draft-sharif-agent-audit-trail-08";
const TRUST_LEVEL: &str = "L1";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AatExportSummary {
    pub dir: String,
    pub sessions: usize,
    pub records: usize,
    pub files: Vec<String>,
    pub draft: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AatVerifyReport {
    pub ok: bool,
    pub records: usize,
    /// Sequence number of the first record whose chain link failed.
    pub broken_at: Option<u64>,
    /// The chain ends with a session_end close record (else "tail unverifiable").
    pub tail_closed: bool,
    pub problems: Vec<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_raw(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// JCS (RFC 8785) canonical bytes of a record with the signature fields and
/// the detached batch removed — what the chain hashes and what a verifier
/// recomputes.
fn canonical_for_hash(record: &Value) -> AppResult<Vec<u8>> {
    let mut v = record.clone();
    if let Some(m) = v.as_object_mut() {
        m.remove("signature");
        m.remove("signature_classical");
        m.remove("batch");
    }
    serde_jcs::to_vec(&v).map_err(|e| AppError::Other(format!("canonicalize: {e}")))
}

pub fn record_hash(record: &Value) -> AppResult<String> {
    Ok(sha256_hex(&canonical_for_hash(record)?))
}

/// A UUID v4-shaped id derived from a digest: deterministic (the same
/// ledger exports to the same ids) and conformant in shape.
fn uuid_from(seed: &str) -> String {
    let d = sha256_raw(seed.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&d[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// RFC 3339 UTC with milliseconds from epoch milliseconds, no chrono.
pub fn rfc3339_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

struct Mapped {
    action_type: &'static str,
    outcome: &'static str,
    phase: &'static str,
}

fn map_kind(e: &ProofEvent) -> Mapped {
    let p = &e.payload;
    let failed = p.get("failed").and_then(Value::as_bool) == Some(true);
    match e.kind.as_str() {
        "approval_request" => Mapped {
            action_type: "escalation",
            outcome: "escalated",
            phase: "pre_execution",
        },
        "approval_decision" => {
            let allow = p.get("decision").and_then(Value::as_str) == Some("allow");
            Mapped {
                action_type: "decision",
                outcome: if allow { "success" } else { "denied" },
                phase: "pre_execution",
            }
        }
        "tool_result" | "check_run" => {
            let bad = failed
                || p.get("exitCode")
                    .and_then(Value::as_i64)
                    .is_some_and(|c| c != 0)
                || p.get("status").and_then(Value::as_str) == Some("failed");
            Mapped {
                action_type: "tool_response",
                outcome: if bad { "failure" } else { "success" },
                phase: "post_execution",
            }
        }
        "agent_action" => {
            let ok = p.get("ok").and_then(Value::as_bool) != Some(false);
            Mapped {
                action_type: "tool_call",
                outcome: if ok { "success" } else { "denied" },
                phase: "post_execution",
            }
        }
        "budget_reached" => Mapped {
            action_type: "escalation",
            outcome: "escalated",
            phase: "concurrent",
        },
        "budget_decision" => {
            let stop = p.get("decision").and_then(Value::as_str) == Some("stop");
            Mapped {
                action_type: "decision",
                outcome: if stop { "denied" } else { "success" },
                phase: "post_execution",
            }
        }
        "agent_declined" => Mapped {
            action_type: "escalation",
            outcome: "escalated",
            phase: "post_execution",
        },
        "prompt" => Mapped {
            action_type: "decision",
            outcome: "success",
            phase: "concurrent",
        },
        "turn_end" => {
            let interrupted = p.get("interrupted").and_then(Value::as_bool) == Some(true);
            Mapped {
                action_type: "lifecycle",
                outcome: if failed {
                    "failure"
                } else if interrupted {
                    "timeout"
                } else {
                    "success"
                },
                phase: "post_execution",
            }
        }
        _ => Mapped {
            action_type: "lifecycle",
            outcome: "success",
            phase: "concurrent",
        },
    }
}

fn principal_for(actor: &str) -> Value {
    if let Some(caller) = actor.strip_prefix("agent:") {
        json!({ "principal_type": "agent", "principal_id": format!("urn:agent-console:agent:{caller}"), "authority_ref": null })
    } else {
        json!({ "principal_type": "human", "principal_id": "urn:agent-console:user:local", "authority_ref": null })
    }
}

/// Build one AAT session chain from a session's Testigo events (already in
/// ledger order). Returns the records and the model last reported.
fn build_session(term: &str, events: &[&ProofEvent], app_version: &str) -> AppResult<Vec<Value>> {
    let agent_id = format!("urn:agent-console:session:{term}");
    let session_id = uuid_from(&format!("session:{term}"));
    let component = format!("urn:agent-console:console:{app_version}");
    let first_ts = events.first().map(|e| e.ts).unwrap_or(0);
    let mut records: Vec<Value> = Vec::new();
    let mut prev_hash: Option<String> = None;
    let mut prev_id: Option<String> = None;
    let mut last_ts = first_ts;
    let mut model: Option<String> = None;
    let push = |mut rec: Map<String, Value>,
                records: &mut Vec<Value>,
                prev_hash: &mut Option<String>,
                prev_id: &mut Option<String>|
     -> AppResult<()> {
        let seq = records.len() as u64;
        let id = uuid_from(&format!("record:{term}:{seq}"));
        rec.insert("record_id".into(), json!(id));
        rec.insert("sequence_number".into(), json!(seq));
        rec.insert("parent_record_id".into(), json!(prev_id.clone()));
        rec.insert("prev_hash".into(), json!(prev_hash.clone()));
        let v = Value::Object(rec);
        *prev_hash = Some(record_hash(&v)?);
        *prev_id = Some(id);
        records.push(v);
        Ok(())
    };
    let base = |ts: i64, actor: &str| -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("timestamp".into(), json!(rfc3339_ms(ts)));
        m.insert("agent_id".into(), json!(agent_id));
        m.insert("principal".into(), principal_for(actor));
        m.insert("agent_version".into(), json!(app_version));
        m.insert("session_id".into(), json!(session_id));
        m.insert("trust_level".into(), json!(TRUST_LEVEL));
        m.insert("recording_component".into(), json!(component));
        m
    };

    // Genesis: the ledger's own session_start when it leads, else synthetic.
    let mut it = events.iter().peekable();
    let mut genesis = base(first_ts, "human");
    let mut genesis_detail = json!({ "event": "session_start", "recording_mode": "independent", "testigo_term_id": term });
    if let Some(e) = it.peek() {
        if e.kind == "session_start" {
            let e = it.next().unwrap();
            if let Some(m) = e.payload.get("model").and_then(Value::as_str) {
                model = Some(m.to_string());
            }
            genesis_detail["testigo_seq"] = json!(e.seq);
            genesis_detail["testigo_hash"] = json!(e.hash);
            if let Some(obj) = e.payload.as_object() {
                for (k, v) in obj {
                    genesis_detail[k] = v.clone();
                }
            }
        }
    }
    genesis.insert("action_type".into(), json!("lifecycle"));
    genesis.insert("action_detail".into(), genesis_detail);
    genesis.insert("outcome".into(), json!("success"));
    genesis.insert("record_phase".into(), json!("concurrent"));
    if let Some(m) = &model {
        genesis.insert("model_id".into(), json!(m));
    }
    push(genesis, &mut records, &mut prev_hash, &mut prev_id)?;

    for e in it {
        let m = map_kind(e);
        if let Some(mm) = e
            .payload
            .get("model")
            .or_else(|| e.payload.get("to"))
            .and_then(Value::as_str)
        {
            if e.kind == "session_start" || e.kind == "model_switch" {
                model = Some(mm.to_string());
            }
        }
        // Monotonic timestamps: a hook that carried no time (0) inherits the
        // previous instant and says so.
        let mut ts = e.ts;
        let clamped = ts < last_ts;
        if clamped {
            ts = last_ts;
        }
        last_ts = ts;
        let mut rec = base(ts, &e.actor);
        let mut detail = Map::new();
        detail.insert("event".into(), json!(e.kind));
        detail.insert("testigo_seq".into(), json!(e.seq));
        detail.insert("testigo_hash".into(), json!(e.hash));
        detail.insert("testigo_case_id".into(), json!(e.case_id));
        if let Some(t) = &e.turn_id {
            detail.insert("turn_id".into(), json!(t));
        }
        if let Some(obj) = e.payload.as_object() {
            for (k, v) in obj {
                detail.insert(k.clone(), v.clone());
            }
        } else if !e.payload.is_null() {
            detail.insert("payload".into(), e.payload.clone());
        }
        if clamped {
            detail.insert("timestamp_clamped".into(), json!(true));
        }
        rec.insert("action_type".into(), json!(m.action_type));
        rec.insert("action_detail".into(), Value::Object(detail));
        rec.insert("outcome".into(), json!(m.outcome));
        rec.insert("record_phase".into(), json!(m.phase));
        if let Some(mm) = &model {
            rec.insert("model_id".into(), json!(mm));
        }
        push(rec, &mut records, &mut prev_hash, &mut prev_id)?;
    }

    // Close: session_hash over the raw digests of every prev_hash so far,
    // including this record's own prev_hash.
    let mut concat: Vec<u8> = Vec::new();
    for r in &records {
        if let Some(h) = r.get("prev_hash").and_then(Value::as_str) {
            concat.extend(hex_to_bytes(h));
        }
    }
    if let Some(h) = &prev_hash {
        concat.extend(hex_to_bytes(h));
    }
    let mut close = base(last_ts, "human");
    close.insert("action_type".into(), json!("lifecycle"));
    close.insert(
        "action_detail".into(),
        json!({
            "event": "session_end",
            "session_hash": sha256_hex(&concat),
            "record_count": records.len() + 1,
            "duration_ms": last_ts - first_ts,
        }),
    );
    close.insert("outcome".into(), json!("success"));
    close.insert("record_phase".into(), json!("post_execution"));
    push(close, &mut records, &mut prev_hash, &mut prev_id)?;
    Ok(records)
}

fn hex_to_bytes(h: &str) -> Vec<u8> {
    (0..h.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok())
        .collect()
}

fn safe_stem(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() {
        "unbound".into()
    } else {
        out
    }
}

/// Export a case (or the full ledger) as one JSONL per session under
/// `<dest_dir>/aat/<case or ledger>/`, with an index and a README that
/// states the mapping and how to verify.
pub fn export(
    testigo: &TestigoService,
    project_root: &str,
    case_id: Option<&str>,
    dest_dir: &Path,
    app_version: &str,
) -> AppResult<AatExportSummary> {
    let events = testigo.list(project_root, case_id, None)?;
    if events.is_empty() {
        return Err(AppError::Other(
            "Nothing to export: the ledger is empty".into(),
        ));
    }
    let mut by_term: BTreeMap<String, Vec<&ProofEvent>> = BTreeMap::new();
    for e in &events {
        by_term
            .entry(e.term_id.clone().unwrap_or_else(|| "unbound".into()))
            .or_default()
            .push(e);
    }
    let stem = safe_stem(case_id.unwrap_or("ledger"));
    let dir = dest_dir.join("aat").join(&stem);
    fs::create_dir_all(&dir)?;
    let mut files = Vec::new();
    let mut total = 0usize;
    let mut index = Vec::new();
    for (term, evs) in &by_term {
        let records = build_session(term, evs, app_version)?;
        let path = dir.join(format!("{}.jsonl", safe_stem(term)));
        let tmp = dir.join(format!("{}.jsonl.tmp", safe_stem(term)));
        {
            let mut f = fs::File::create(&tmp)?;
            for r in &records {
                let line = serde_json::to_string(r).map_err(|e| AppError::Other(e.to_string()))?;
                f.write_all(line.as_bytes())?;
                f.write_all(b"\n")?;
            }
            f.flush()?;
        }
        fs::rename(&tmp, &path)?;
        total += records.len();
        index.push(json!({
            "session": term,
            "file": path.file_name().map(|n| n.to_string_lossy().to_string()),
            "records": records.len(),
            "session_id": records[0]["session_id"],
            "tail_hash": record_hash(records.last().unwrap())?,
        }));
        files.push(path.to_string_lossy().to_string());
    }
    let index_path = dir.join("index.json");
    fs::write(
        &index_path,
        serde_json::to_string_pretty(&json!({
            "format": "IETF Agent Audit Trail (JSONL)",
            "draft": AAT_DRAFT,
            "exported_by": format!("agent-console {app_version}"),
            "project": project_root,
            "case_id": case_id,
            "trust_level": TRUST_LEVEL,
            "sessions": index,
        }))
        .map_err(|e| AppError::Other(e.to_string()))?,
    )?;
    files.push(index_path.to_string_lossy().to_string());
    let readme = dir.join("README.md");
    fs::write(&readme, README)?;
    files.push(readme.to_string_lossy().to_string());
    let verifier = dir.join("aat-verify.mjs");
    fs::write(&verifier, VERIFIER_MJS)?;
    files.push(verifier.to_string_lossy().to_string());
    Ok(AatExportSummary {
        dir: dir.to_string_lossy().to_string(),
        sessions: by_term.len(),
        records: total,
        files,
        draft: AAT_DRAFT.into(),
    })
}

/// The draft's verification procedure (Section 6.3) over one JSONL file:
/// genesis shape, recomputed prev_hash links, parent ids, monotonic
/// timestamps, gap-free sequence numbers, and whether the tail is a close.
pub fn verify_file(path: &Path) -> AppResult<AatVerifyReport> {
    let f = fs::File::open(path)?;
    let mut problems = Vec::new();
    let mut prev: Option<Value> = None;
    let mut n = 0u64;
    let mut broken_at = None;
    let mut last_ts = String::new();
    let mut last: Option<Value> = None;
    let mut prev_hashes: Vec<String> = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: Value = serde_json::from_str(&line)
            .map_err(|e| AppError::Other(format!("line {}: {e}", n + 1)))?;
        for k in [
            "record_id",
            "timestamp",
            "agent_id",
            "principal",
            "agent_version",
            "session_id",
            "action_type",
            "action_detail",
            "outcome",
            "trust_level",
            "record_phase",
        ] {
            if rec.get(k).is_none() {
                problems.push(format!("record {n}: missing {k}"));
            }
        }
        if let Some(sn) = rec.get("sequence_number").and_then(Value::as_u64) {
            if sn != n {
                problems.push(format!("record {n}: sequence_number {sn} is not gap-free"));
            }
        }
        match &prev {
            None => {
                if !rec["parent_record_id"].is_null() || !rec["prev_hash"].is_null() {
                    problems.push(
                        "genesis record must have null parent_record_id and prev_hash".into(),
                    );
                }
                if rec["action_type"] != "lifecycle"
                    || rec["action_detail"]["event"] != "session_start"
                {
                    problems.push("genesis record must be lifecycle/session_start".into());
                }
            }
            Some(p) => {
                let expected = record_hash(p)?;
                if rec["prev_hash"].as_str() != Some(expected.as_str()) {
                    problems.push(format!(
                        "record {n}: prev_hash does not match the previous record"
                    ));
                    if broken_at.is_none() {
                        broken_at = Some(n);
                    }
                }
                if rec["parent_record_id"] != p["record_id"] {
                    problems.push(format!(
                        "record {n}: parent_record_id does not name the previous record"
                    ));
                }
            }
        }
        if let Some(h) = rec["prev_hash"].as_str() {
            prev_hashes.push(h.to_string());
        }
        let ts = rec["timestamp"].as_str().unwrap_or("").to_string();
        if ts < last_ts {
            problems.push(format!("record {n}: timestamp goes backwards"));
        }
        last_ts = ts;
        prev = Some(rec.clone());
        last = Some(rec);
        n += 1;
    }
    let tail_closed = last.as_ref().is_some_and(|r| {
        r["action_type"] == "lifecycle" && r["action_detail"]["event"] == "session_end"
    });
    if !tail_closed {
        problems.push("tail unverifiable: the last record is not a session_end close".into());
    }
    Ok(AatVerifyReport {
        ok: problems.is_empty(),
        records: n as usize,
        broken_at,
        tail_closed,
        problems,
    })
}

pub fn verify_dir(dir: &Path) -> AppResult<Vec<(PathBuf, AatVerifyReport)>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let r = verify_file(&p)?;
            out.push((p, r));
        }
    }
    Ok(out)
}

/// The dependency-free Node verifier written next to every export, so the
/// receiver checks the chain without Agent Console (or any install beyond node).
const VERIFIER_MJS: &str = include_str!("../../resources/aat-verify.mjs");

const README: &str = r#"# Agent Audit Trail export (IETF draft-sharif-agent-audit-trail-08)

One `.jsonl` file per session. Each line is one audit record; records are
hash-chained: `prev_hash` is the lowercase hex SHA-256 of the previous record
serialized with the JSON Canonicalization Scheme (RFC 8785), with the
`signature`, `signature_classical` and `batch` fields removed. The first record
is the `lifecycle/session_start` genesis (`prev_hash` and `parent_record_id`
null); the last is the `lifecycle/session_end` close with `session_hash`,
`record_count` and `duration_ms`. `sequence_number` is gap-free from 0.

Verify offline, no Agent Console needed:

    node aat-verify.mjs *.jsonl

(`aat-verify.mjs` is written alongside; it has no dependencies.) Any
conforming verifier works too — for example the `last-record` web verifier —
or by hand: recompute each record's hash and compare it with the next
record's `prev_hash`; check parent ids, monotonic timestamps, that the tail
is a close, and that the close's `session_hash` is the SHA-256 over the raw
32-byte `prev_hash` digests of every record in order.

Mapping from the Testigo ledger (the console's native chain):

| Testigo kind | action_type | record_phase | outcome |
| --- | --- | --- | --- |
| session_start | lifecycle (genesis) | concurrent | success |
| prompt | decision | concurrent | success |
| approval_request | escalation | pre_execution | escalated |
| approval_decision | decision | pre_execution | success / denied |
| tool_result, check_run | tool_response | post_execution | success / failure |
| turn_end | lifecycle | post_execution | success / failure / timeout |
| agent_action (MCP) | tool_call | post_execution | success / denied |
| agent_declined (MCP) | escalation | post_execution | escalated |
| budget_reached | escalation | concurrent | escalated |
| budget_decision | decision | post_execution | success / denied (stop) |
| anything else | lifecycle | concurrent | success |

`action_detail` keeps the Testigo `event`, `testigo_seq`, `testigo_hash`,
`testigo_case_id` and the original payload, so both chains cross-reference.
`principal` is the human the agent acts for, or `agent:<name>` for calls the
agent made through the console's MCP server. Trust level is L1 everywhere:
identified, independently recorded, not cryptographically attested.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_and_uuid_shape() {
        assert_eq!(rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_ms(1_792_245_600_123), "2026-10-17T14:00:00.123Z");
        let u = uuid_from("x");
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(uuid_from("x"), uuid_from("x"));
    }

    #[test]
    fn canonical_hash_excludes_signatures_and_batch_and_is_key_order_independent() {
        let a = json!({ "b": 1, "a": "x", "signature": "zzz", "batch": { "x": 1 } });
        let b = json!({ "a": "x", "b": 1 });
        assert_eq!(record_hash(&a).unwrap(), record_hash(&b).unwrap());
        assert_eq!(record_hash(&b).unwrap(), sha256_hex(br#"{"a":"x","b":1}"#));
    }

    fn ev(seq: u64, ts: i64, kind: &str, term: &str, actor: &str, payload: Value) -> ProofEvent {
        ProofEvent {
            seq,
            ts,
            case_id: "case-1".into(),
            turn_id: Some("t1".into()),
            kind: kind.into(),
            term_id: Some(term.into()),
            session_id: None,
            actor: actor.into(),
            payload,
            prev_hash: "0".repeat(64),
            hash: format!("{seq:064x}"),
        }
    }

    #[test]
    fn a_session_exports_to_a_verifiable_chain_and_tampering_breaks_it() {
        let events = vec![
            ev(
                1,
                1_000,
                "session_start",
                "s1",
                "agent",
                json!({ "model": "claude-opus-5-5" }),
            ),
            ev(
                2,
                2_000,
                "prompt",
                "s1",
                "human",
                json!({ "prompt": "fix it" }),
            ),
            ev(
                3,
                0,
                "approval_request",
                "s1",
                "agent",
                json!({ "tool": "Bash" }),
            ),
            ev(
                4,
                3_000,
                "approval_decision",
                "s1",
                "human",
                json!({ "decision": "deny" }),
            ),
            ev(
                5,
                4_000,
                "tool_result",
                "s1",
                "agent",
                json!({ "tool": "Bash", "exitCode": 1 }),
            ),
            ev(
                6,
                5_000,
                "agent_action",
                "s1",
                "agent:claude-desktop",
                json!({ "tool": "send_prompt", "ok": true }),
            ),
            ev(
                7,
                5_500,
                "budget_reached",
                "s1",
                "console",
                json!({ "spentUsd": 12.3, "capUsd": 10.0, "action": "interrupted" }),
            ),
            ev(
                8,
                5_600,
                "budget_decision",
                "s1",
                "human",
                json!({ "decision": "stop", "spentUsd": 12.3, "capUsd": 10.0 }),
            ),
            ev(
                9,
                6_000,
                "turn_end",
                "s1",
                "agent",
                json!({ "summary": "done" }),
            ),
        ];
        let refs: Vec<&ProofEvent> = events.iter().collect();
        let records = build_session("s1", &refs, "0.89.0").unwrap();
        assert_eq!(records.len(), 10, "genesis + 8 events + close");
        assert_eq!(records[6]["action_type"], "escalation");
        assert_eq!(records[7]["outcome"], "denied");
        assert_eq!(records[0]["action_detail"]["event"], "session_start");
        assert_eq!(records[0]["model_id"], "claude-opus-5-5");
        assert!(records[0]["prev_hash"].is_null() && records[0]["parent_record_id"].is_null());
        assert_eq!(records[2]["action_type"], "escalation");
        assert_eq!(records[2]["action_detail"]["timestamp_clamped"], true);
        assert_eq!(records[3]["outcome"], "denied");
        assert_eq!(records[4]["outcome"], "failure");
        assert_eq!(records[5]["principal"]["principal_type"], "agent");
        assert_eq!(records[5]["action_type"], "tool_call");
        let close = records.last().unwrap();
        assert_eq!(close["action_detail"]["event"], "session_end");
        assert_eq!(close["action_detail"]["record_count"], 10);
        for i in 1..records.len() {
            assert_eq!(
                records[i]["prev_hash"],
                record_hash(&records[i - 1]).unwrap()
            );
            assert_eq!(records[i]["parent_record_id"], records[i - 1]["record_id"]);
            assert_eq!(records[i]["sequence_number"], i as u64);
        }

        let dir = std::env::temp_dir().join(format!(
            "ac-aat-{}-{}",
            std::process::id(),
            records[0]["record_id"].as_str().unwrap()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s1.jsonl");
        let mut body = String::new();
        for r in &records {
            body.push_str(&serde_json::to_string(r).unwrap());
            body.push('\n');
        }
        fs::write(&path, &body).unwrap();
        let ok = verify_file(&path).unwrap();
        assert!(ok.ok, "{:?}", ok.problems);
        assert_eq!(ok.records, 10);
        assert!(ok.tail_closed);

        // Flip one character of record 4's payload: the link from 5 breaks.
        let tampered = body.replacen("\"decision\":\"deny\"", "\"decision\":\"allow\"", 1);
        assert_ne!(tampered, body);
        fs::write(&path, &tampered).unwrap();
        let bad = verify_file(&path).unwrap();
        assert!(!bad.ok);
        assert_eq!(bad.broken_at, Some(4));

        // The shipped Node verifier agrees with the Rust one, both ways
        // (skipped where node is not on PATH; CI has it).
        let verifier = dir.join("aat-verify.mjs");
        fs::write(&verifier, VERIFIER_MJS).unwrap();
        let run = |expect_ok: bool| match std::process::Command::new("node")
            .arg(&verifier)
            .arg(&path)
            .output()
        {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                assert_eq!(
                    out.status.success(),
                    expect_ok,
                    "node verifier said: {text}"
                );
            }
            Err(_) => eprintln!("node not on PATH: skipping the cross-check"),
        };
        run(false);
        fs::write(&path, &body).unwrap();
        run(true);
        fs::remove_dir_all(&dir).ok();
    }
}
