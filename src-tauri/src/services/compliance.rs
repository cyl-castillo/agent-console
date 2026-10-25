//! F11 (COMPETENCIA-2026-10): the compliance pack — one reproducible zip
//! an auditor can take away: the IETF Agent Audit Trail export of the
//! ledger (hash-chained JSONL, verifier alongside), the approval policy in
//! force with its hash, the console's verification verdicts summarized by
//! week, and a manifest with the SHA-256 of every file.
//!
//! Reproducible: the same ledger and policy produce the same bytes. Nothing
//! in the pack reads the wall clock — timestamps come from the ledger, the
//! AAT record ids are digests, zip entries carry a fixed modification time
//! and are written in sorted order. An auditor who re-runs the export over
//! the same ledger gets the same SHA-256.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::aat_export::{self, rfc3339_ms, sha256_hex, AAT_DRAFT};
use crate::services::policy_service;
use crate::services::testigo_service::{ProofEvent, TestigoService};

pub const PACK_FORMAT: &str = "agent-console compliance pack";
pub const PACK_VERSION: u32 = 1;

/// One week of verification verdicts, Monday-based; `week_start` is the
/// Monday's date (UTC) so the row needs no calendar library to read.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WeekRow {
    pub week_start: String,
    pub turns: u32,
    pub verified: u32,
    pub checked: u32,
    pub no_claim: u32,
    pub unverified_claim: u32,
    pub stale_check: u32,
    pub claim_contradicted: u32,
    pub unwitnessed: u32,
    /// `verification_decision {accept}` lines: verdicts a human accepted on
    /// their word.
    pub accepted_by_human: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceSummary {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub draft: String,
    pub sessions: usize,
    pub records: usize,
    /// Every AAT chain in the pack passed the draft's verification procedure.
    pub aat_ok: bool,
    pub weeks: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_hash: Option<String>,
    pub files: Vec<String>,
    /// The ledger's last sequence number the pack covers.
    pub last_seq: u64,
}

/// The Monday (UTC) of the week `ms` falls in, as `YYYY-MM-DD`.
pub fn week_start(ms: i64) -> String {
    let day = ms.div_euclid(86_400_000);
    // 1970-01-01 was a Thursday: Monday-based weekday = (day + 3) mod 7.
    let weekday = (day + 3).rem_euclid(7);
    let monday = day - weekday;
    rfc3339_ms(monday * 86_400_000)[..10].to_string()
}

/// The verdict rows per week, oldest first. A `verification` line counts
/// once by its status; the last verdict of a turn wins when a turn was
/// re-verified.
pub fn weekly_verification(events: &[ProofEvent]) -> Vec<WeekRow> {
    // Last verdict per turn (turn_id → event), then per week.
    let mut last: BTreeMap<String, &ProofEvent> = BTreeMap::new();
    let mut unbound: Vec<&ProofEvent> = Vec::new();
    for e in events.iter().filter(|e| e.kind == "verification") {
        match &e.turn_id {
            Some(t) => {
                last.insert(t.clone(), e);
            }
            None => unbound.push(e),
        }
    }
    let mut weeks: BTreeMap<String, WeekRow> = BTreeMap::new();
    for e in last.values().copied().chain(unbound) {
        let row = weeks.entry(week_start(e.ts)).or_default();
        row.turns += 1;
        match e
            .payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
        {
            "verified" => row.verified += 1,
            "checked" => row.checked += 1,
            "no_claim" => row.no_claim += 1,
            "unverified_claim" => row.unverified_claim += 1,
            "stale_check" => row.stale_check += 1,
            "claim_contradicted" => row.claim_contradicted += 1,
            _ => row.unwitnessed += 1,
        }
    }
    for e in events.iter().filter(|e| {
        e.kind == "verification_decision"
            && e.payload.get("decision").and_then(Value::as_str) == Some("accept")
    }) {
        weeks.entry(week_start(e.ts)).or_default().accepted_by_human += 1;
    }
    weeks
        .into_iter()
        .map(|(k, mut r)| {
            r.week_start = k;
            r
        })
        .collect()
}

pub fn weekly_csv(rows: &[WeekRow]) -> String {
    let mut s = String::from(
        "week_start,turns,verified,checked,no_claim,unverified_claim,stale_check,claim_contradicted,unwitnessed,accepted_by_human\n",
    );
    for r in rows {
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            r.week_start,
            r.turns,
            r.verified,
            r.checked,
            r.no_claim,
            r.unverified_claim,
            r.stale_check,
            r.claim_contradicted,
            r.unwitnessed,
            r.accepted_by_human
        ));
    }
    s
}

fn safe_stem(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if t.is_empty() {
        "ledger".into()
    } else {
        t
    }
}

/// Every regular file under `root`, as (path relative to root with `/`,
/// absolute path), sorted — the order the zip is written in.
fn walk(root: &Path) -> AppResult<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    fn rec(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> AppResult<()> {
        for entry in fs::read_dir(dir)? {
            let p = entry?.path();
            if p.is_dir() {
                rec(base, &p, out)?;
            } else {
                let rel = p
                    .strip_prefix(base)
                    .map_err(|e| AppError::Other(e.to_string()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, p));
            }
        }
        Ok(())
    }
    rec(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

const PACK_README: &str = r#"# Compliance pack

What an auditor needs, in one archive that any party can re-verify:

- `aat/<case>/*.jsonl` — the ledger as an IETF Agent Audit Trail
  (one hash-chained JSONL file per session), with `aat-verify.mjs` and the
  export README alongside. Verify offline: `node aat/<case>/aat-verify.mjs aat/<case>/*.jsonl`.
- `aat/<case>/verify-report.json` — the console's own run of that
  verification at pack time (every chain must be `ok`).
- `policy/policy.json` + `policy/policy.sha256` — the approval policy in
  force when the pack was made; the same hash rides on every
  `approval_decision` line (`policyHash`), so a decision can be read against
  the rule that governed it. Absent when the project has no policy file.
- `verification-weekly.json` / `.csv` — the console's verdict on every turn's
  own claims (verified / checked / no_claim / unverified_claim / stale_check /
  claim_contradicted / unwitnessed), per Monday-based week, plus how many
  verdicts a human accepted on their word.
- `manifest.json` — format, draft, app version, the ledger's last sequence
  number and time, the policy hash, and the SHA-256 of every file above.

Reproducible: the same ledger and policy produce the same bytes (ledger
timestamps only, digest-derived record ids, fixed zip times, sorted
entries). Compare the archive's SHA-256 with the one the console reported.
"#;

/// Build the pack for a project (or one case) under `dest_dir/compliance/`.
pub fn pack(
    testigo: &TestigoService,
    project_root: &str,
    case_id: Option<&str>,
    dest_dir: &Path,
    app_version: &str,
) -> AppResult<ComplianceSummary> {
    let events = testigo.list(project_root, case_id, None)?;
    if events.is_empty() {
        return Err(AppError::Other(
            "Nothing to pack: the ledger is empty".into(),
        ));
    }
    let last_seq = events.iter().map(|e| e.seq).max().unwrap_or(0);
    let last_ts = events.iter().map(|e| e.ts).max().unwrap_or(0);
    let stem = safe_stem(case_id.unwrap_or("ledger"));
    let out_dir = dest_dir.join("compliance");
    fs::create_dir_all(&out_dir)?;
    let build = out_dir.join(format!(".build-{stem}-{}", std::process::id()));
    if build.exists() {
        fs::remove_dir_all(&build)?;
    }
    fs::create_dir_all(&build)?;
    let result = build_into(
        testigo,
        project_root,
        case_id,
        &build,
        app_version,
        &events,
        last_seq,
        last_ts,
    );
    let _ = fs::remove_dir_all(&build);
    let (zip_bytes, files, aat, aat_ok, policy_hash, weeks) = result?;
    let path = out_dir.join(format!("compliance-{stem}.zip"));
    let tmp = out_dir.join(format!("compliance-{stem}.zip.tmp"));
    fs::write(&tmp, &zip_bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(ComplianceSummary {
        path: path.to_string_lossy().to_string(),
        bytes: zip_bytes.len() as u64,
        sha256: sha256_hex(&zip_bytes),
        draft: AAT_DRAFT.into(),
        sessions: aat.sessions,
        records: aat.records,
        aat_ok,
        weeks,
        policy_hash,
        files,
        last_seq,
    })
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn build_into(
    testigo: &TestigoService,
    project_root: &str,
    case_id: Option<&str>,
    build: &Path,
    app_version: &str,
    events: &[ProofEvent],
    last_seq: u64,
    last_ts: i64,
) -> AppResult<(
    Vec<u8>,
    Vec<String>,
    aat_export::AatExportSummary,
    bool,
    Option<String>,
    usize,
)> {
    // 1. The AAT export, then the draft's verification over it — the gate.
    let aat = aat_export::export(testigo, project_root, case_id, build, app_version)?;
    let aat_dir = PathBuf::from(&aat.dir);
    let reports = aat_export::verify_dir(&aat_dir)?;
    let aat_ok = !reports.is_empty() && reports.iter().all(|(_, r)| r.ok);
    let report_json: Vec<Value> = reports
        .iter()
        .map(|(p, r)| {
            json!({
                "file": p.file_name().map(|n| n.to_string_lossy().to_string()),
                "ok": r.ok,
                "records": r.records,
                "brokenAt": r.broken_at,
                "tailClosed": r.tail_closed,
                "problems": r.problems,
            })
        })
        .collect();
    fs::write(
        aat_dir.join("verify-report.json"),
        serde_json::to_string_pretty(
            &json!({ "draft": AAT_DRAFT, "allOk": aat_ok, "chains": report_json }),
        )
        .map_err(|e| AppError::Other(e.to_string()))?,
    )?;
    if !aat_ok {
        return Err(AppError::Other(
            "the audit trail did not verify; the pack was not written".into(),
        ));
    }

    // 2. The policy in force, when the project has one.
    let policy_hash = match policy_service::load(Path::new(project_root))? {
        Some(p) => {
            let dir = build.join("policy");
            fs::create_dir_all(&dir)?;
            let h = policy_service::hash(&p);
            fs::write(
                dir.join("policy.json"),
                serde_json::to_string_pretty(&p).map_err(|e| AppError::Other(e.to_string()))?,
            )?;
            fs::write(dir.join("policy.sha256"), format!("{h}\n"))?;
            Some(h)
        }
        None => None,
    };

    // 3. Verification by week.
    let rows = weekly_verification(events);
    fs::write(
        build.join("verification-weekly.json"),
        serde_json::to_string_pretty(&rows).map_err(|e| AppError::Other(e.to_string()))?,
    )?;
    fs::write(build.join("verification-weekly.csv"), weekly_csv(&rows))?;
    fs::write(build.join("README.md"), PACK_README)?;

    // 4. The manifest: every file's digest, then the archive.
    let entries = walk(build)?;
    let mut manifest_files = Vec::new();
    for (rel, abs) in &entries {
        let bytes = fs::read(abs)?;
        manifest_files
            .push(json!({ "path": rel, "sha256": sha256_hex(&bytes), "bytes": bytes.len() }));
    }
    let manifest = json!({
        "format": PACK_FORMAT,
        "version": PACK_VERSION,
        "draft": AAT_DRAFT,
        "appVersion": app_version,
        "project": project_root,
        "caseId": case_id,
        "lastSeq": last_seq,
        "lastTs": rfc3339_ms(last_ts),
        "policyHash": policy_hash,
        "aat": { "sessions": aat.sessions, "records": aat.records, "ok": aat_ok },
        "files": manifest_files,
    });
    fs::write(
        build.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).map_err(|e| AppError::Other(e.to_string()))?,
    )?;
    let entries = walk(build)?;
    let files: Vec<String> = entries.iter().map(|(rel, _)| rel.clone()).collect();

    let mut buf: Vec<u8> = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut z = zip::ZipWriter::new(cursor);
        // A fixed time keeps the archive reproducible; the ledger's own
        // times are inside the records.
        let fixed = zip::DateTime::from_date_and_time(2026, 1, 1, 0, 0, 0)
            .map_err(|e| AppError::Other(format!("zip time: {e:?}")))?;
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(fixed);
        for (rel, abs) in &entries {
            z.start_file(rel.as_str(), opts)
                .map_err(|e| AppError::Other(format!("zip entry {rel}: {e}")))?;
            z.write_all(&fs::read(abs)?)?;
        }
        z.finish()
            .map_err(|e| AppError::Other(format!("zip finish: {e}")))?;
    }
    Ok((buf, files, aat, aat_ok, policy_hash, rows.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn ev(seq: u64, ts: i64, kind: &str, turn: Option<&str>, payload: Value) -> ProofEvent {
        ProofEvent {
            seq,
            ts,
            case_id: "c".into(),
            turn_id: turn.map(String::from),
            kind: kind.into(),
            term_id: Some("t1".into()),
            session_id: None,
            actor: "console".into(),
            payload,
            prev_hash: String::new(),
            hash: String::new(),
        }
    }

    #[test]
    fn weeks_start_on_monday_utc() {
        // 2026-10-17 is a Saturday → its week starts Monday 2026-10-12.
        assert_eq!(week_start(1_792_245_600_123), "2026-10-12");
        // 1970-01-01 (Thursday) → Monday 1969-12-29.
        assert_eq!(week_start(0), "1969-12-29");
        // A Monday maps to itself.
        assert_eq!(week_start(1_791_813_600_000), "2026-10-12");
    }

    #[test]
    fn weekly_rows_count_the_last_verdict_per_turn_and_human_accepts() {
        let w1 = 1_792_245_600_000; // 2026-10-17
        let w2 = w1 + 7 * 86_400_000;
        let events = vec![
            ev(
                1,
                w1,
                "verification",
                Some("a"),
                json!({"status": "unverified_claim"}),
            ),
            ev(
                2,
                w1 + 1000,
                "verification",
                Some("a"),
                json!({"status": "verified"}),
            ),
            ev(
                3,
                w1,
                "verification",
                Some("b"),
                json!({"status": "claim_contradicted"}),
            ),
            ev(
                4,
                w1,
                "verification_decision",
                Some("b"),
                json!({"decision": "accept", "verificationSeq": 3}),
            ),
            ev(
                5,
                w1,
                "verification_decision",
                Some("b"),
                json!({"decision": "evidence_requested"}),
            ),
            ev(
                6,
                w2,
                "verification",
                Some("c"),
                json!({"status": "no_claim"}),
            ),
            ev(
                7,
                w2,
                "verification",
                None,
                json!({"status": "unwitnessed"}),
            ),
        ];
        let rows = weekly_verification(&events);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].week_start, "2026-10-12");
        assert_eq!(
            (rows[0].turns, rows[0].verified, rows[0].unverified_claim),
            (2, 1, 0),
            "last verdict wins"
        );
        assert_eq!(rows[0].claim_contradicted, 1);
        assert_eq!(rows[0].accepted_by_human, 1);
        assert_eq!(rows[1].week_start, "2026-10-19");
        assert_eq!(
            (rows[1].turns, rows[1].no_claim, rows[1].unwitnessed),
            (2, 1, 1)
        );
        let csv = weekly_csv(&rows);
        assert!(csv.starts_with("week_start,turns,"));
        assert!(csv.contains("2026-10-12,2,1,0,0,0,0,1,0,1\n"));
    }

    /// The gate: the pack holds the AAT export (and verifies it), the policy
    /// with its hash, the weekly rows and a manifest whose digests match;
    /// building it twice over the same ledger gives the same bytes; the
    /// shipped Node verifier passes over the chains inside.
    #[test]
    fn pack_is_complete_verified_and_reproducible() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-compliance-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", base.join("data"));
        let project = base.join("proj");
        fs::create_dir_all(project.join(".agent-console")).unwrap();
        fs::write(
            project.join(".agent-console/policy.json"),
            r#"{"version":1,"rules":[{"effect":"allow","tool":"shell","pattern":"npm test"}]}"#,
        )
        .unwrap();
        let root = project.to_string_lossy().to_string();
        let svc = TestigoService::new();
        let p = svc
            .on_prompt(
                &root,
                1_792_245_600_000,
                Some("t1"),
                Some("s1"),
                Some("do it"),
                None,
                None,
            )
            .unwrap();
        let turn = p.turn_id.clone().unwrap();
        svc.on_turn_end(
            &root,
            1_792_245_700_000,
            Some("t1"),
            Some("s1"),
            json!({ "summary": "All tests pass.", "filesChanged": [] }),
        )
        .unwrap();
        let _ = turn;
        let events = svc.list(&root, None, None).unwrap();
        assert!(
            events.iter().any(|e| e.kind == "verification"),
            "the turn got its verdict"
        );

        let dest = base.join("out");
        let a = pack(&svc, &root, None, &dest, "0.0.0-test").unwrap();
        assert!(a.aat_ok);
        assert!(a.policy_hash.is_some());
        assert!(a.weeks >= 1);
        assert!(Path::new(&a.path).exists());
        for f in [
            "manifest.json",
            "README.md",
            "verification-weekly.json",
            "verification-weekly.csv",
            "policy/policy.json",
            "policy/policy.sha256",
        ] {
            assert!(
                a.files.iter().any(|x| x == f),
                "missing {f} in {:?}",
                a.files
            );
        }
        assert!(a
            .files
            .iter()
            .any(|x| x.starts_with("aat/ledger/") && x.ends_with(".jsonl")));
        assert!(a.files.iter().any(|x| x == "aat/ledger/verify-report.json"));
        assert!(a.files.iter().any(|x| x == "aat/ledger/aat-verify.mjs"));

        // Read the archive back: manifest digests match the entries.
        let file = fs::File::open(&a.path).unwrap();
        let mut z = zip::ZipArchive::new(file).unwrap();
        let mut read = |name: &str| -> Vec<u8> {
            let mut e = z.by_name(name).unwrap();
            let mut v = Vec::new();
            std::io::Read::read_to_end(&mut e, &mut v).unwrap();
            v
        };
        let manifest: Value = serde_json::from_slice(&read("manifest.json")).unwrap();
        assert_eq!(manifest["format"], PACK_FORMAT);
        assert_eq!(manifest["draft"], AAT_DRAFT);
        assert_eq!(manifest["policyHash"], json!(a.policy_hash));
        for f in manifest["files"].as_array().unwrap() {
            let name = f["path"].as_str().unwrap();
            let bytes = read(name);
            assert_eq!(f["sha256"], json!(sha256_hex(&bytes)), "digest of {name}");
        }
        let policy_hash_file = String::from_utf8(read("policy/policy.sha256")).unwrap();
        assert_eq!(policy_hash_file.trim(), a.policy_hash.as_deref().unwrap());

        // Reproducible: the same ledger packs to the same bytes.
        let bytes_a = fs::read(&a.path).unwrap();
        let b = pack(&svc, &root, None, &dest, "0.0.0-test").unwrap();
        let bytes_b = fs::read(&b.path).unwrap();
        assert_eq!(bytes_a, bytes_b);
        assert_eq!(a.sha256, b.sha256);

        // The shipped Node verifier passes over the chains inside the pack
        // (skipped where node is not on PATH; CI has it).
        let extract = base.join("extract");
        fs::create_dir_all(&extract).unwrap();
        for i in 0..z.len() {
            let mut e = z.by_index(i).unwrap();
            let name = e.name().unwrap().to_string();
            let out = extract.join(&name);
            fs::create_dir_all(out.parent().unwrap()).unwrap();
            let mut f = fs::File::create(&out).unwrap();
            std::io::copy(&mut e, &mut f).unwrap();
        }
        let aat_dir = extract.join("aat/ledger");
        let chains: Vec<PathBuf> = fs::read_dir(&aat_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        assert!(!chains.is_empty());
        match std::process::Command::new("node")
            .arg(aat_dir.join("aat-verify.mjs"))
            .args(&chains)
            .output()
        {
            Ok(out) => assert!(
                out.status.success(),
                "node verifier: {}",
                String::from_utf8_lossy(&out.stdout)
            ),
            Err(_) => eprintln!("node not on PATH: skipping the cross-check"),
        }
    }
}
