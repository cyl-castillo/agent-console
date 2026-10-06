//! Folders a project's sessions may run in besides the project checkout.
//!
//! A session can be started in another folder (a second repo, a sibling
//! service) without opening a second app instance. The git, snapshot and file
//! commands follow the active session's folder, and they must never be aimed
//! at whatever path the webview sends: a path from the UI is data, not
//! authority. So the only way a folder gets onto this list is the native
//! folder picker run by the backend (`commands::session_folder::folder_pick`),
//! and the commands accept a folder only if it is on the list for the open
//! project.
//!
//! One small JSON file per project under
//! `<data_local>/agent-console/linked-folders/<project key>.json`, written
//! atomically with a `.bak`. A file that can't be read fails closed: no folder
//! is authorized until the user picks it again. Nothing on the list is ever
//! deleted from disk; this only records which folders the user chose.

use std::fs;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::persistence::project_file_key;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkedFolder {
    /// Canonical absolute path, as the picker resolved it.
    pub path: String,
    /// Last path component, for the session chip and the chooser.
    pub name: String,
    pub last_used_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LinkedFoldersFile {
    #[serde(default)]
    folders: Vec<LinkedFolder>,
}

/// Enough for every repo someone works across from one project; the oldest
/// pick falls off first.
const MAX_FOLDERS: usize = 20;

static LOCK: Mutex<()> = Mutex::new(());

fn dir() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("no data_local dir".into()))?
        .join("agent-console")
        .join("linked-folders");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn file_path(project_root: &str) -> AppResult<PathBuf> {
    Ok(dir()?.join(format!("{}.json", project_file_key(project_root))))
}

/// The main file, else its `.bak`, else empty. Never an error: an unreadable
/// allowlist authorizes nothing, which is the safe reading.
fn load_file(path: &Path) -> LinkedFoldersFile {
    let parse = |p: &Path| -> Option<LinkedFoldersFile> {
        let txt = fs::read_to_string(p).ok()?;
        serde_json::from_str(&txt).ok()
    };
    parse(path)
        .or_else(|| parse(&path.with_extension("json.bak")))
        .unwrap_or_default()
}

fn write_file(path: &Path, file: &LinkedFoldersFile) -> AppResult<()> {
    let json = serde_json::to_string_pretty(file)
        .map_err(|e| AppError::Other(format!("serialize linked folders: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
    if path.exists() {
        let _ = fs::copy(path, path.with_extension("json.bak"));
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The project's linked folders, most recently used first. Folders that no
/// longer exist are left out (and stay authorized nowhere, since `is_linked`
/// requires the directory to exist).
pub fn list(project_root: &str) -> AppResult<Vec<LinkedFolder>> {
    let path = file_path(project_root)?;
    let _g = LOCK.lock();
    Ok(load_file(&path)
        .folders
        .into_iter()
        .filter(|f| Path::new(&f.path).is_dir())
        .collect())
}

/// Record a folder the user picked for this project. It must be an existing
/// directory; the stored path is canonical so `is_linked` can compare by
/// identity. Picking a folder again moves it to the front.
pub fn link(project_root: &str, folder: &Path) -> AppResult<LinkedFolder> {
    if !folder.is_dir() {
        return Err(AppError::InvalidArgument(format!(
            "'{}' is not a folder",
            folder.display()
        )));
    }
    let canon = canonical(folder);
    let entry = LinkedFolder {
        path: canon.to_string_lossy().to_string(),
        name: canon
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| canon.to_string_lossy().to_string()),
        last_used_ms: now_ms(),
    };
    let path = file_path(project_root)?;
    let _g = LOCK.lock();
    let mut file = load_file(&path);
    file.folders
        .retain(|f| canonical(Path::new(&f.path)) != canon);
    file.folders.insert(0, entry.clone());
    file.folders.truncate(MAX_FOLDERS);
    write_file(&path, &file)?;
    Ok(entry)
}

/// Remove a folder from the project's list. Only takes authority away, so the
/// path may come from the UI. Matches the stored spelling or the same
/// directory under another spelling; a folder that no longer exists is still
/// removable by its stored path. Returns whether anything was removed.
pub fn unlink(project_root: &str, folder: &Path) -> AppResult<bool> {
    let canon = canonical(folder);
    let path = file_path(project_root)?;
    let _g = LOCK.lock();
    let mut file = load_file(&path);
    let before = file.folders.len();
    file.folders.retain(|f| {
        let stored = Path::new(&f.path);
        stored != folder && canonical(stored) != canon
    });
    if file.folders.len() == before {
        return Ok(false);
    }
    write_file(&path, &file)?;
    Ok(true)
}

/// Whether `folder` is an existing directory the user linked to this project.
pub fn is_linked(project_root: &str, folder: &Path) -> bool {
    if !folder.is_dir() {
        return false;
    }
    let canon = canonical(folder);
    list(project_root)
        .map(|v| v.iter().any(|f| canonical(Path::new(&f.path)) == canon))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d =
            std::env::temp_dir().join(format!("ac-linked-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// One test fn on purpose: it mutates the process-global XDG_DATA_HOME.
    #[test]
    fn allowlist_round_trip_is_per_project_and_fails_closed() {
        let _env = crate::test_support::lock_env();
        let xdg = temp_dir("xdg");
        std::env::set_var("XDG_DATA_HOME", &xdg);

        let project = temp_dir("project");
        let other_project = temp_dir("other-project");
        let backend = temp_dir("backend");
        let stranger = temp_dir("stranger");
        let root = project.to_string_lossy().to_string();
        let other_root = other_project.to_string_lossy().to_string();

        // Nothing is linked until the user picks it.
        assert!(list(&root).unwrap().is_empty());
        assert!(!is_linked(&root, &backend));

        // A picked folder is authorized for that project only.
        let linked = link(&root, &backend).unwrap();
        assert_eq!(linked.path, canonical(&backend).to_string_lossy());
        assert!(is_linked(&root, &backend));
        assert!(!is_linked(&root, &stranger), "unpicked folder stays out");
        assert!(!is_linked(&other_root, &backend), "lists are per project");

        // A different spelling of the same directory is the same folder.
        let dotted = backend.join("..").join(backend.file_name().unwrap());
        assert!(is_linked(&root, &dotted));

        // Re-picking moves it to the front without duplicating.
        link(&root, &stranger).unwrap();
        link(&root, &dotted).unwrap();
        let names: Vec<String> = list(&root).unwrap().into_iter().map(|f| f.path).collect();
        assert_eq!(
            names,
            vec![
                canonical(&backend).to_string_lossy().to_string(),
                canonical(&stranger).to_string_lossy().to_string()
            ]
        );

        // A file path is not a folder.
        let file = backend.join("README.md");
        fs::write(&file, "x").unwrap();
        assert!(link(&root, &file).is_err());

        // Corrupt main file: the .bak (written by the previous save) answers.
        let path = file_path(&root).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert!(is_linked(&root, &backend), "falls back to the .bak");

        // Both corrupt: nothing is authorized, and nothing panics.
        fs::write(path.with_extension("json.bak"), "also not json").unwrap();
        assert!(list(&root).unwrap().is_empty());
        assert!(
            !is_linked(&root, &backend),
            "unreadable allowlist fails closed"
        );

        // A save after corruption starts a clean list.
        link(&root, &backend).unwrap();
        assert!(is_linked(&root, &backend));

        // A linked folder that disappears is no longer authorized.
        fs::remove_dir_all(&stranger).unwrap();
        link(&root, &backend).unwrap();
        assert!(!is_linked(&root, &stranger));

        // Unlinking takes the folder off this project's list only, by any
        // spelling of it; a second unlink is a no-op.
        link(&other_root, &backend).unwrap();
        let extra = temp_dir("extra");
        link(&root, &extra).unwrap();
        assert!(unlink(&root, &dotted).unwrap());
        assert!(!is_linked(&root, &backend), "unlinked");
        assert!(is_linked(&root, &extra), "the rest of the list stays");
        assert!(is_linked(&other_root, &backend), "other projects untouched");
        assert!(!unlink(&root, &backend).unwrap(), "already gone");

        // A folder deleted from disk is still removable by its stored path.
        let extra_stored = canonical(&extra);
        fs::remove_dir_all(&extra).unwrap();
        assert!(unlink(&root, &extra_stored).unwrap());
        assert!(load_file(&file_path(&root).unwrap()).folders.is_empty());

        for d in [&xdg, &project, &other_project, &backend] {
            let _ = fs::remove_dir_all(d);
        }
    }
}
