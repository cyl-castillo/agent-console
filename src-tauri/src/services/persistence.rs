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
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::error::{AppError, AppResult};

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

/// `<file>.tmp` next to `path`: same directory, so the rename that follows
/// never crosses a filesystem.
fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Rename, retrying briefly. On Windows, antivirus and indexers lock a file
/// that was just written and the rename fails for a moment; without the
/// retry every save in that window is lost (issue #72).
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..3 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(80));
                }
            }
        }
    }
    Err(last.expect("loop ran"))
}

/// Write the bytes and flush them to disk before returning. Without the
/// flush, a power cut after the rename can leave the new name pointing at an
/// empty file — which is how a store ends up with a blank main file.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Replace `path` with `bytes` so that a reader, or a crash at any point,
/// sees either the old content or the new one — never a half-written file.
/// No-op when the file already holds exactly these bytes.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    let tmp = tmp_sibling(path);
    write_synced(&tmp, bytes)?;
    rename_with_retry(&tmp, path)
}

fn is_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(u8::is_ascii_whitespace)
}

fn parses_as_json(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde::de::IgnoredAny>(bytes).is_ok()
}

/// Marker inside the error a store returns for a file written by a newer
/// build. Callers that recover from unreadable files (quarantine, start
/// fresh) must not do so for this one: see `is_newer_schema`.
pub const NEWER_SCHEMA: &str = "was written by a newer Agent Console";

/// True when `e` is the "written by a newer build" refusal.
pub fn is_newer_schema(e: &AppError) -> bool {
    e.to_string().contains(NEWER_SCHEMA)
}

/// The top-level `schemaVersion` of a store file, without building the whole
/// document. Absent in files written before stores were versioned: those
/// are schema 1 by definition.
fn schema_version_of(txt: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct Peek {
        #[serde(default, rename = "schemaVersion")]
        schema_version: Option<u64>,
    }
    serde_json::from_str::<Peek>(txt).ok()?.schema_version
}

/// The store's own fields with `schemaVersion` written first.
#[derive(serde::Serialize)]
struct Stamped<'a, T: serde::Serialize> {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(flatten)]
    store: &'a T,
}

/// Load a JSON store that keeps a `.bak` of its last good state.
///
/// - No main file: the store has never been written → `T::default()`.
/// - Main file blank (truncated by a crash or a full disk) or unparseable:
///   that is damage, not "no data" — recover from the `.bak`. A blank main
///   with no usable `.bak` is an empty store; an unparseable one is an error,
///   so the caller cannot mistake unreadable history for none and write over
///   it.
pub fn load_json_store<T: DeserializeOwned + Default>(
    path: &Path,
    bak: &Path,
    name: &str,
    schema: Option<u32>,
) -> AppResult<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let txt = fs::read_to_string(path).map_err(|e| AppError::Other(format!("read {name}: {e}")))?;
    // A file from a NEWER version may hold fields this build would drop on
    // the next save. Refuse it — and do not fall back to the .bak, which
    // would bring older data forward and then overwrite the newer file.
    if let (Some(supported), Some(found)) = (schema, schema_version_of(&txt)) {
        if found > u64::from(supported) {
            return Err(AppError::Other(format!(
                "{name} {NEWER_SCHEMA} (schema {found}, this build reads up to {supported}); \
                 it was left untouched"
            )));
        }
    }
    let from_bak = || {
        fs::read_to_string(bak)
            .ok()
            .filter(|b| !b.trim().is_empty())
            .and_then(|b| serde_json::from_str::<T>(&b).ok())
    };
    if txt.trim().is_empty() {
        return Ok(match from_bak() {
            Some(file) => {
                tracing::warn!("{name} was blank; recovered from its backup");
                file
            }
            None => T::default(),
        });
    }
    match serde_json::from_str::<T>(&txt) {
        Ok(file) => Ok(file),
        Err(e) => match from_bak() {
            Some(file) => {
                tracing::warn!("{name} did not parse ({e}); recovered from its backup");
                Ok(file)
            }
            None => Err(AppError::Other(format!("parse {name}: {e}"))),
        },
    }
}

/// Save a JSON store atomically, keeping the previous state as `.bak`.
/// With `schema`, the file carries that number as its top-level
/// `schemaVersion`.
///
/// The `.bak` is the last resort of `load_json_store`, so it is only ever
/// replaced by a main file that is itself good: a blank or unparseable main
/// (the very thing the backup exists for) is never copied over it. No-op
/// when nothing changed, so a periodic save does not rewrite the file.
pub fn save_json_store<T: serde::Serialize>(
    path: &Path,
    bak: &Path,
    tmp: &Path,
    name: &str,
    schema: Option<u32>,
    store: &T,
) -> AppResult<()> {
    let json = match schema {
        Some(schema_version) => serde_json::to_string_pretty(&Stamped {
            schema_version,
            store,
        }),
        None => serde_json::to_string_pretty(store),
    }
    .map_err(|e| AppError::Other(format!("serialize {name}: {e}")))?;
    let current = fs::read(path).ok();
    if current.as_deref() == Some(json.as_bytes()) {
        return Ok(());
    }
    write_synced(tmp, json.as_bytes())?;
    if let Some(cur) = &current {
        if !is_blank(cur) && parses_as_json(cur) {
            let _ = fs::copy(path, bak);
        }
    }
    rename_with_retry(tmp, path)?;
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

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Store {
        #[serde(default)]
        items: Vec<String>,
    }

    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("ac-persist-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn paths(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
        (
            dir.join("s.json"),
            dir.join("s.json.bak"),
            dir.join("s.json.tmp"),
        )
    }

    fn store(items: &[&str]) -> Store {
        Store {
            items: items.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_blank_main_file_is_recovered_from_the_backup_and_never_replaces_it() {
        let dir = scratch("blank");
        let (path, bak, tmp) = paths(&dir);
        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &store(&["one"])).unwrap();
        save_json_store(
            &path,
            &bak,
            &tmp,
            "s.json",
            Some(1),
            &store(&["one", "two"]),
        )
        .unwrap();
        // A crash truncated the main file.
        fs::write(&path, "").unwrap();

        let loaded: Store = load_json_store(&path, &bak, "s.json", Some(1)).unwrap();
        assert_eq!(
            loaded.items,
            vec!["one"],
            "the last good state, from the .bak"
        );

        // The next save must not push the blank file over the good backup.
        save_json_store(
            &path,
            &bak,
            &tmp,
            "s.json",
            Some(1),
            &store(&["one", "three"]),
        )
        .unwrap();
        let kept: Store = serde_json::from_str(&fs::read_to_string(&bak).unwrap()).unwrap();
        assert_eq!(kept.items, vec!["one"], ".bak still holds the good state");
        let now: Store = load_json_store(&path, &bak, "s.json", Some(1)).unwrap();
        assert_eq!(now.items, vec!["one", "three"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_main_file_never_replaces_the_backup_either() {
        let dir = scratch("corrupt");
        let (path, bak, tmp) = paths(&dir);
        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &store(&["a"])).unwrap();
        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &store(&["a", "b"])).unwrap();
        fs::write(&path, "{ half a wri").unwrap();

        let loaded: Store = load_json_store(&path, &bak, "s.json", Some(1)).unwrap();
        assert_eq!(loaded.items, vec!["a"]);
        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &store(&["a", "c"])).unwrap();
        assert!(
            parses_as_json(&fs::read(&bak).unwrap()),
            ".bak is still good JSON"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_tells_no_data_from_unreadable_data() {
        let dir = scratch("states");
        let (path, bak, _tmp) = paths(&dir);
        // Never written: empty store.
        assert_eq!(
            load_json_store::<Store>(&path, &bak, "s.json", Some(1)).unwrap(),
            Store::default()
        );
        // Blank, no backup: empty store.
        fs::write(&path, "  \n").unwrap();
        assert_eq!(
            load_json_store::<Store>(&path, &bak, "s.json", Some(1)).unwrap(),
            Store::default()
        );
        // Corrupt, no usable backup: an error, never a silent empty store.
        fs::write(&path, "{ nope").unwrap();
        fs::write(&bak, "also nope").unwrap();
        assert!(load_json_store::<Store>(&path, &bak, "s.json", Some(1)).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_the_same_content_does_not_touch_the_file() {
        let dir = scratch("same");
        let (path, bak, tmp) = paths(&dir);
        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &store(&["x"])).unwrap();
        assert!(!bak.exists(), "first save has nothing to back up");
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &store(&["x"])).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
        assert!(!bak.exists(), "an unchanged save makes no backup");
        assert!(!tmp.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_replaces_the_file_and_leaves_no_temp_behind() {
        let dir = scratch("atomic");
        let path = dir.join("config.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert!(!tmp_sibling(&path).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_that_cannot_finish_leaves_the_old_file_whole() {
        let dir = scratch("fail");
        let path = dir.join("config.json");
        write_atomic(&path, b"good").unwrap();
        // The temp name is taken by a directory: the write fails before the
        // rename, as a full disk or a permission error would make it.
        fs::create_dir(tmp_sibling(&path)).unwrap();
        assert!(write_atomic(&path, b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"good");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_store_carries_its_schema_version_and_old_files_read_as_schema_one() {
        let dir = scratch("schema");
        let (path, bak, tmp) = paths(&dir);
        // A file from before stores were versioned: no schemaVersion at all.
        fs::write(&path, r#"{"items":["from 0.90"]}"#).unwrap();
        let old: Store = load_json_store(&path, &bak, "s.json", Some(1)).unwrap();
        assert_eq!(old.items, vec!["from 0.90"]);

        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &old).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["schemaVersion"], 1);
        assert_eq!(
            raw["items"][0], "from 0.90",
            "the store's own fields stay top-level"
        );
        let again: Store = load_json_store(&path, &bak, "s.json", Some(1)).unwrap();
        assert_eq!(again, old);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_from_a_newer_build_is_refused_and_left_untouched() {
        let dir = scratch("newer");
        let (path, bak, tmp) = paths(&dir);
        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &store(&["old"])).unwrap();
        save_json_store(&path, &bak, &tmp, "s.json", Some(1), &store(&["old", "er"])).unwrap();
        let newer = r#"{"schemaVersion":2,"items":["x"],"fieldThisBuildDoesNotKnow":true}"#;
        fs::write(&path, newer).unwrap();

        let err = load_json_store::<Store>(&path, &bak, "s.json", Some(1)).unwrap_err();
        assert!(is_newer_schema(&err), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), newer, "not rewritten");
        // An ordinary parse failure is NOT mistaken for a newer file.
        fs::write(&path, "{ nope").unwrap();
        fs::write(&bak, "nope").unwrap();
        let plain = load_json_store::<Store>(&path, &bak, "s.json", Some(1)).unwrap_err();
        assert!(!is_newer_schema(&plain));
        let _ = fs::remove_dir_all(&dir);
    }
}
