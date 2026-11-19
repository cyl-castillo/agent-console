//! The ledger's line format across versions: a ledger written before lines
//! declared a `schemaVersion` keeps verifying, a ledger that mixes both
//! formats verifies, and a ledger from a newer build is never written to.
//!
//! The files under `fixtures/ledgers/` are frozen bytes. `v1.jsonl` comes from
//! `make-v1.py`, an implementation of the format that does not call the app.
//! The rest is regenerated with:
//!
//!   cargo test --lib regenerate_ledger_fixtures -- --ignored

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::services::testigo_export::export_with_seed;
use crate::services::testigo_service::{
    recompute_line_hash, ProofEvent, TestigoService, LEDGER_SCHEMA_VERSION,
};

const V1: &str = include_str!("../../fixtures/ledgers/v1.jsonl");
const MIXED: &str = include_str!("../../fixtures/ledgers/mixed-v1-v2.jsonl");
const CASE: &str = "jira:DEMO-7";
/// A throwaway key for fixtures; it signs nothing but these files.
const FIXTURE_SEED: [u8; 32] = [7u8; 32];

fn isolated(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "ac-ledger-schema-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&base).unwrap();
    std::env::set_var("XDG_DATA_HOME", &base);
    base
}

fn seed_ledger(root: &str, content: &str) -> PathBuf {
    let path = TestigoService::ledger_path(root).unwrap();
    fs::write(&path, content).unwrap();
    path
}

/// A well-formed line in a format this build does not know yet.
fn future_line(seq: u64, prev: &str) -> String {
    let body = format!(
        "{{\"schemaVersion\":{},\"seq\":{seq},\"ts\":9000,\"caseId\":\"unbound\",\
         \"kind\":\"something_new\",\"actor\":\"system\",\"payload\":{{}},\
         \"prevHash\":\"{prev}\",\"hash\":\"\"}}",
        LEDGER_SCHEMA_VERSION + 1
    );
    let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
    body.replace("\"hash\":\"\"", &format!("\"hash\":\"{hash}\""))
}

/// One test fn on purpose: it moves the process-global `XDG_DATA_HOME`.
#[test]
fn ledgers_of_every_line_format_verify_and_a_newer_one_is_left_alone() {
    let _env = crate::test_support::lock_env();
    let base = isolated("read");

    // 1. A ledger as every build up to v0.98 wrote it.
    let root = "/proj/v1";
    let path = seed_ledger(root, V1);
    let svc = TestigoService::new();
    let rep = svc.verify(root).unwrap();
    assert!(rep.ok && rep.broken_at_seq.is_none(), "v1 chain verifies");
    assert_eq!(rep.total, V1.lines().count());
    assert!(rep.newer_schema_at_seq.is_none());
    // Reading a v1 line and writing it back does not change a byte, so
    // nothing that round-trips a record can break its hash.
    for line in V1.lines() {
        let ev: ProofEvent = serde_json::from_str(line).unwrap();
        assert_eq!(ev.schema_version, 1);
        assert_eq!(serde_json::to_string(&ev).unwrap(), line);
        assert_eq!(recompute_line_hash(line).as_deref(), Some(ev.hash.as_str()));
    }

    // 2. This build appends to it: the new line says which format it is, the
    // old lines are untouched, and the mixed chain verifies.
    svc.on_prompt(root, 6000, Some("t1"), None, Some("next"), None, None)
        .unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.starts_with(V1), "existing lines are never rewritten");
    let last = after.lines().last().unwrap();
    assert!(
        last.starts_with(&format!(
            "{{\"schemaVersion\":{LEDGER_SCHEMA_VERSION},\"seq\":"
        )),
        "{last}"
    );
    assert!(
        last.trim_end().ends_with("\"}"),
        "hash stays the last member"
    );
    // The version is covered by the hash like every other byte.
    let relabelled = last.replacen("\"schemaVersion\":2", "\"schemaVersion\":1", 1);
    assert_ne!(&relabelled, last);
    let appended: ProofEvent = serde_json::from_str(last).unwrap();
    assert_ne!(
        recompute_line_hash(&relabelled).as_deref(),
        Some(appended.hash.as_str())
    );
    let rep = svc.verify(root).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.total, V1.lines().count() + 1);

    // 3. The committed mixed ledger (frozen v2 bytes) verifies too.
    let mixed_root = "/proj/mixed";
    seed_ledger(mixed_root, MIXED);
    let svc = TestigoService::new();
    let rep = svc.verify(mixed_root).unwrap();
    assert!(rep.ok, "mixed v1+v2 chain verifies: {rep:?}");
    assert_eq!(rep.total, MIXED.lines().count());
    assert!(MIXED.starts_with(V1));
    assert!(MIXED.lines().count() > V1.lines().count());

    // 4. A newer build wrote the last line. The chain still links and
    // hashes, verify says where the unknown format starts, and this build
    // refuses to append after it — without touching the file.
    let future_root = "/proj/future";
    let tail = V1.lines().last().unwrap();
    let tail: ProofEvent = serde_json::from_str(tail).unwrap();
    let future = format!("{V1}{}\n", future_line(tail.seq + 1, &tail.hash));
    let path = seed_ledger(future_root, &future);
    let svc = TestigoService::new();
    let rep = svc.verify(future_root).unwrap();
    assert!(rep.ok, "the bytes still hash and link");
    assert_eq!(rep.newer_schema_at_seq, Some(tail.seq + 1));
    let err = svc
        .on_prompt(future_root, 9500, Some("t1"), None, Some("x"), None, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("newer Agent Console"), "{err}");
    assert_eq!(fs::read_to_string(&path).unwrap(), future);

    // 5. A newer line this build cannot even parse is not a crash-torn tail:
    // it must not be cut off.
    let odd_root = "/proj/odd";
    let odd = format!(
        "{V1}{{\"schemaVersion\":{},\"shape\":\"unknown\"}}\n",
        LEDGER_SCHEMA_VERSION + 1
    );
    let path = seed_ledger(odd_root, &odd);
    let svc = TestigoService::new();
    assert!(svc
        .on_prompt(odd_root, 9500, Some("t1"), None, Some("x"), None, None)
        .is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), odd, "nothing truncated");

    let _ = fs::remove_dir_all(&base);
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ledgers")
}

/// Rewrites the mixed ledger and both packets from `v1.jsonl`.
#[test]
#[ignore]
fn regenerate_ledger_fixtures() {
    let _env = crate::test_support::lock_env();
    let base = isolated("regen");
    let out = fixtures_dir();
    let pack = |root: &str, name: &str| {
        let svc = TestigoService::new();
        let dest = base.join(name);
        let sum =
            export_with_seed(&svc, root, Some(CASE), &dest, &FIXTURE_SEED, &[], None).unwrap();
        fs::copy(&sum.path, out.join(name)).unwrap();
    };

    seed_ledger("/proj/v1", V1);
    pack("/proj/v1", "v1.proofpack.json");

    let root = "/proj/mixed";
    let path = seed_ledger(root, V1);
    let svc = TestigoService::new();
    svc.on_prompt(
        root,
        6000,
        Some("t1"),
        Some("s-0001"),
        Some("y ahora el lint"),
        None,
        None,
    )
    .unwrap();
    svc.on_turn_end(
        root,
        7000,
        Some("t1"),
        Some("s-0001"),
        serde_json::json!({}),
    )
    .unwrap();
    fs::copy(&path, out.join("mixed-v1-v2.jsonl")).unwrap();
    pack(root, "mixed.proofpack.json");
    let _ = fs::remove_dir_all(&base);
}
