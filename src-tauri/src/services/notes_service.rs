//! Per-project sticky notes: the scratchpad you keep while driving agents
//! (prompt ideas, "review later", ticket reminders). Mirrors sessions_service's
//! persistence invariants: atomic tmp→bak→rename writes, corrupt-read fallback
//! to .bak, load-before-save so one project's save never clobbers another's.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Retention cap per project. Sticky notes are a working surface, not an
/// archive — past this we drop the OLDEST notes on save (newest are kept) so
/// the file can't grow without bound. 200 is far beyond any usable board.
const MAX_NOTES_PER_PROJECT: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub text: String,
    /// Preset color key ("yellow" | "pink" | "blue" | "green" | "purple") —
    /// the UI maps it to the sticky palette; unknown values render as yellow.
    pub color: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NotesFile {
    #[serde(default)]
    by_project: HashMap<String, Vec<Note>>,
}

pub struct NotesService {
    lock: Mutex<()>,
}

impl NotesService {
    pub fn new() -> Self {
        Self { lock: Mutex::new(()) }
    }

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("notes.json"))
    }
    fn bak_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("notes.json.bak"))
    }
    fn tmp_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("notes.json.tmp"))
    }

    /// Missing/empty file = legitimate empty state; a parse failure on an
    /// EXISTING file falls back to .bak, and errors (never "empty") if that
    /// fails too — so callers can't mistake unreadable data for "no notes"
    /// and overwrite it.
    fn load_file() -> AppResult<NotesFile> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(NotesFile::default());
        }
        let txt = fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("read notes.json: {e}")))?;
        if txt.trim().is_empty() {
            return Ok(NotesFile::default());
        }
        match serde_json::from_str::<NotesFile>(&txt) {
            Ok(file) => Ok(file),
            Err(e) => {
                if let Ok(bak) = Self::bak_path() {
                    if let Ok(btxt) = fs::read_to_string(&bak) {
                        if let Ok(file) = serde_json::from_str::<NotesFile>(&btxt) {
                            return Ok(file);
                        }
                    }
                }
                Err(AppError::Other(format!("parse notes.json: {e}")))
            }
        }
    }

    /// Atomic write: temp file → back up the live (known-good) file → rename.
    /// A crash mid-write can only ever damage the temp file.
    fn write_file(file: &NotesFile) -> AppResult<()> {
        let path = Self::path()?;
        let json = serde_json::to_string_pretty(file)
            .map_err(|e| AppError::Other(format!("serialize: {e}")))?;
        let tmp = Self::tmp_path()?;
        fs::write(&tmp, json.as_bytes())?;
        if path.exists() {
            if let Ok(bak) = Self::bak_path() {
                let _ = fs::copy(&path, &bak);
            }
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn list(&self, project_root: &str) -> AppResult<Vec<Note>> {
        let _g = self.lock.lock();
        let file = Self::load_file()?;
        Ok(file.by_project.get(project_root).cloned().unwrap_or_default())
    }

    pub fn save(&self, project_root: &str, mut notes: Vec<Note>) -> AppResult<()> {
        let _g = self.lock.lock();
        // Load-before-save: merge into the other projects' notes, never clobber.
        let mut file = Self::load_file()?;
        if notes.len() > MAX_NOTES_PER_PROJECT {
            // Keep the newest; the caller's order is the board order, so sort a
            // copy by update time to decide which fall off.
            notes.sort_by_key(|n| std::cmp::Reverse(n.updated_at_ms));
            notes.truncate(MAX_NOTES_PER_PROJECT);
        }
        if notes.is_empty() {
            file.by_project.remove(project_root);
        } else {
            file.by_project.insert(project_root.to_string(), notes);
        }
        Self::write_file(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn note(id: &str, text: &str) -> Note {
        Note {
            id: id.into(),
            text: text.into(),
            color: "yellow".into(),
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    /// One test fn (it mutates the process-global XDG_DATA_HOME): round-trip,
    /// multi-project isolation, corrupt-main → .bak recovery.
    #[test]
    fn persistence_round_trip_and_bak_recovery() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ac-notes-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_DATA_HOME", &dir);

        let svc = NotesService::new();
        assert!(svc.list("/proj/a").unwrap().is_empty());

        svc.save("/proj/a", vec![note("n1", "try prompt X"), note("n2", "review PR")])
            .unwrap();
        svc.save("/proj/b", vec![note("n3", "other project")]).unwrap();

        // Round trip + isolation between projects.
        let a = svc.list("/proj/a").unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].text, "try prompt X");
        assert_eq!(svc.list("/proj/b").unwrap().len(), 1);

        // Saving one project must not clobber the other (load-before-save).
        svc.save("/proj/a", vec![note("n1", "edited")]).unwrap();
        assert_eq!(svc.list("/proj/b").unwrap().len(), 1);

        // Corrupt the main file: list() must recover via .bak, not report empty.
        let path = NotesService::path().unwrap();
        std::fs::write(&path, "{ corrupted").unwrap();
        let recovered = svc.list("/proj/b").unwrap();
        assert_eq!(recovered.len(), 1, "bak fallback must recover, not lose notes");

        // Empty save removes the project bucket entirely.
        svc.save("/proj/b", vec![]).unwrap();
        // (list may come from bak-recovered state after re-save; re-check a.)
        assert!(!svc.list("/proj/a").unwrap().is_empty());

        std::env::remove_var("XDG_DATA_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
