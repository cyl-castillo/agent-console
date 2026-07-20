//! Shared per-project persistence primitives (MEJORAS-2026-07 R3.b).
//!
//! Before this module, `key_for` lived copy-pasted in activity, testigo and
//! scheduler (byte-identical ×3), and the JSONL trim in two of them — ~200
//! lines of drift-prone duplication. Extracted verbatim: the outputs of
//! `project_file_key` NAME users' on-disk ledgers, so any change to it
//! orphans their history. The golden test below pins real observed filenames.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::error::AppResult;

/// Stable per-project filename: a human-readable prefix (last path segment)
/// plus a hash of the full root, so two projects sharing a basename never
/// collide and the file is still recognizable on disk.
///
/// ⚠ Output stability is a DATA contract: these keys name existing users'
/// activity/testigo/scheduler files. Never change the algorithm without a
/// migration. (Known design debt, predating this module: DefaultHasher's
/// algorithm is not formally guaranteed across Rust releases; it has been
/// stable in practice, and the golden test will scream if a toolchain bump
/// ever breaks it — at which point a migration will be needed anyway.)
pub fn project_file_key(project_root: &str) -> String {
    let mut h = DefaultHasher::new();
    project_root.hash(&mut h);
    let hash = h.finish();
    let last = project_root
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("root");
    let clean: String = last
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(24)
        .collect();
    format!("{clean}-{hash:016x}.jsonl")
}

/// Rewrite a JSONL file keeping only the most recent `keep` non-empty lines,
/// atomically (temp file + rename) so a crash mid-trim can't truncate live
/// history. No-op when the file is already within bounds.
pub fn trim_jsonl(path: &Path, keep: usize) -> AppResult<()> {
    let content = fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= keep {
        return Ok(());
    }
    let kept = lines[lines.len() - keep..].join("\n");
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, format!("{kept}\n"))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GOLDEN: these two filenames exist on real installs (they name the
    /// author's actual demo and fixy ledgers). If this test ever fails, the
    /// key algorithm's output changed — which orphans every user's on-disk
    /// history. Do not "fix" the expected values; write a migration.
    #[test]
    fn project_file_key_matches_observed_filenames() {
        assert_eq!(
            project_file_key("/home/father/Documents/workspaces/demo-playground"),
            "demo_playground-e96ab40fafb7bf7d.jsonl"
        );
        assert_eq!(
            project_file_key("/home/father/Documents/workspaces/fixy-backend"),
            "fixy_backend-bef2972e2b6bb9da.jsonl"
        );
        // Shape invariants for arbitrary paths.
        let k = project_file_key("C:\\Proj\\My App!");
        assert!(k.starts_with("My_App_-") && k.ends_with(".jsonl"));
        assert_eq!(project_file_key("x"), project_file_key("x"));
        assert_ne!(project_file_key("/a/proj"), project_file_key("/b/proj"));
    }

    #[test]
    fn trim_jsonl_keeps_newest_atomically() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("ac-trim-{}-{nanos}.jsonl", std::process::id()));
        fs::write(&path, "{\"n\":1}\n{\"n\":2}\n\n{\"n\":3}\n").unwrap();

        trim_jsonl(&path, 2).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(
            after, "{\"n\":2}\n{\"n\":3}\n",
            "keeps the newest, drops blanks"
        );

        // Within bounds: byte-identical no-op.
        trim_jsonl(&path, 10).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), after);

        let _ = fs::remove_file(&path);
    }
}
