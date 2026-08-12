//! Outcome signals for the memory corpus (E2 of the knowledge flywheel).
//!
//! Per-project, per-document usefulness stats: how often a doc was injected
//! (passive, recorded by the inject endpoint) and what the user verdicted
//! about those injections (explicit 👍/👎 — the only signal trusted to move
//! rankings; automatic outcome attribution is guesswork and stays out).
//!
//! Effect on retrieval — INJECTION ONLY, manual search is never reweighted:
//! - a bounded nudge (±NUDGE_CAP) on the cosine score, so sustained verdicts
//!   break ties but can never overturn a clearly better semantic match;
//! - a hard exclusion once a doc collects EXCLUDE_AFTER unhelpfuls with not a
//!   single helpful — it stops being injected (still searchable, still
//!   visible in the GUI as excluded, and one click rehabilitates it).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::persistence::project_file_key;

/// Per-verdict score step. Ten net verdicts saturate the nudge.
const NUDGE_STEP: f32 = 0.003;
/// The nudge can break ties, never arguments: well under the 0.74 threshold
/// band and typical score gaps between distinct topics.
pub const NUDGE_CAP: f32 = 0.03;
/// Unhelpful verdicts (with zero helpfuls) after which a doc stops being
/// injected. Three clicks is deliberate: one misfire shouldn't bury a doc.
pub const EXCLUDE_AFTER: u32 = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocStats {
    #[serde(default)]
    pub injected_count: u32,
    #[serde(default)]
    pub helpful: u32,
    #[serde(default)]
    pub unhelpful: u32,
    #[serde(default)]
    pub last_injected_ms: u64,
}

impl DocStats {
    /// Bounded score adjustment from net verdicts.
    pub fn nudge(&self) -> f32 {
        let net = self.helpful as f32 - self.unhelpful as f32;
        (net * NUDGE_STEP).clamp(-NUDGE_CAP, NUDGE_CAP)
    }

    /// Hard stop: repeatedly voted useless and never once useful.
    pub fn excluded(&self) -> bool {
        self.unhelpful >= EXCLUDE_AFTER && self.helpful == 0
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FeedbackFile {
    #[serde(default)]
    docs: HashMap<String, DocStats>,
}

fn feedback_dir() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console")
        .join("memory-feedback");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn file_path(project_root: &str) -> AppResult<PathBuf> {
    // Same key scheme as the semantic index — the two files describe the same
    // corpus and should sit recognizably side by side on disk.
    Ok(feedback_dir()?.join(format!("{}.json", project_file_key(project_root))))
}

fn load(project_root: &str) -> FeedbackFile {
    let Ok(path) = file_path(project_root) else {
        return FeedbackFile::default();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return FeedbackFile::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(project_root: &str, file: &FeedbackFile) -> AppResult<()> {
    let path = file_path(project_root)?;
    let raw = serde_json::to_string(file).map_err(|e| AppError::Other(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// All stats for a project, keyed by doc id ("memory:release.md", "skill:…").
pub fn stats(project_root: &str) -> HashMap<String, DocStats> {
    load(project_root).docs
}

/// Passive usage signal: these docs were just injected. Best-effort — the
/// inject path must never fail over bookkeeping.
pub fn record_injected(project_root: &str, doc_ids: &[String], now_ms: u64) {
    if doc_ids.is_empty() {
        return;
    }
    let mut file = load(project_root);
    for id in doc_ids {
        let s = file.docs.entry(id.clone()).or_default();
        s.injected_count += 1;
        s.last_injected_ms = now_ms;
    }
    let _ = save(project_root, &file);
}

/// Explicit user verdict on one doc.
pub fn set_verdict(project_root: &str, doc_id: &str, helpful: bool) -> AppResult<DocStats> {
    let mut file = load(project_root);
    let s = file.docs.entry(doc_id.to_string()).or_default();
    if helpful {
        s.helpful += 1;
    } else {
        s.unhelpful += 1;
    }
    let out = s.clone();
    save(project_root, &file)?;
    Ok(out)
}

/// Rehabilitation: wipe the verdicts (usage history stays). The user's manual
/// override always wins over accumulated clicks.
pub fn reset_verdicts(project_root: &str, doc_id: &str) -> AppResult<DocStats> {
    let mut file = load(project_root);
    let s = file.docs.entry(doc_id.to_string()).or_default();
    s.helpful = 0;
    s.unhelpful = 0;
    let out = s.clone();
    save(project_root, &file)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudge_is_bounded_and_signed() {
        let mut s = DocStats::default();
        assert_eq!(s.nudge(), 0.0);
        s.helpful = 2;
        assert!(s.nudge() > 0.0 && s.nudge() <= NUDGE_CAP);
        s.helpful = 100;
        assert_eq!(s.nudge(), NUDGE_CAP, "saturates, never grows unbounded");
        s.helpful = 0;
        s.unhelpful = 100;
        assert_eq!(s.nudge(), -NUDGE_CAP);
    }

    #[test]
    fn exclusion_needs_repeated_unhelpful_and_zero_helpful() {
        let mut s = DocStats {
            unhelpful: EXCLUDE_AFTER - 1,
            ..Default::default()
        };
        assert!(!s.excluded(), "one misfire short of the bar");
        s.unhelpful = EXCLUDE_AFTER;
        assert!(s.excluded());
        // A single helpful vote lifts the exclusion — mixed signal ≠ useless.
        s.helpful = 1;
        assert!(!s.excluded());
    }

    #[test]
    fn store_roundtrip_verdicts_and_rehab() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-corpus-fb-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let root = "/proj/fb";
        assert!(stats(root).is_empty(), "fresh project: no stats, no error");

        record_injected(root, &["memory:a.md".into(), "skill:x".into()], 111);
        record_injected(root, &["memory:a.md".into()], 222);
        let all = stats(root);
        assert_eq!(all["memory:a.md"].injected_count, 2);
        assert_eq!(all["memory:a.md"].last_injected_ms, 222);
        assert_eq!(all["skill:x"].injected_count, 1);

        set_verdict(root, "memory:a.md", false).unwrap();
        set_verdict(root, "memory:a.md", false).unwrap();
        let s = set_verdict(root, "memory:a.md", false).unwrap();
        assert!(s.excluded(), "three unhelpful, zero helpful");

        let s = reset_verdicts(root, "memory:a.md").unwrap();
        assert!(!s.excluded());
        assert_eq!(s.injected_count, 2, "usage history survives rehab");

        // Corrupt file degrades to empty, never to an error.
        let path = file_path(root).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert!(stats(root).is_empty());
    }
}
