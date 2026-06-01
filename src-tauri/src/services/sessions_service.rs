use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedSession {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub created_at_ms: u64,
    #[serde(default)]
    pub scrollback: String,
    /// Which coding agent this session launches ("claude" | "codex"). Absent =
    /// Claude (the default, and the only option before agent selection existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Claude Code session id captured from the UserPromptSubmit hook; used to
    /// auto-resume a Claude conversation when the user reactivates this terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    /// True once the rename suggestion has been offered for this session, so
    /// we don't re-suggest on every subsequent prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_suggested: Option<bool>,
    /// Model alias or full id last chosen for this session, replayed as
    /// `claude --model <model>` when the terminal spawns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionsFile {
    #[serde(default)]
    by_project: HashMap<String, Vec<PersistedSession>>,
}

pub struct SessionsService {
    lock: Mutex<()>,
}

impl SessionsService {
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
        Ok(Self::dir()?.join("sessions.json"))
    }

    fn bak_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("sessions.json.bak"))
    }

    fn tmp_path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("sessions.json.tmp"))
    }

    /// Load the sessions file, distinguishing "no history yet" from "could not
    /// read existing history". A missing or empty file is a legitimate empty
    /// state (Ok(default)); a read or parse failure on an EXISTING file is an
    /// error so callers don't mistake unreadable data for "no sessions" and
    /// overwrite it. On parse failure we first try the `.bak` copy.
    fn load_file() -> AppResult<SessionsFile> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(SessionsFile::default());
        }
        let txt = fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("read sessions.json: {e}")))?;
        if txt.trim().is_empty() {
            return Ok(SessionsFile::default());
        }
        match serde_json::from_str::<SessionsFile>(&txt) {
            Ok(file) => Ok(file),
            Err(e) => {
                // Main file is corrupt — fall back to the last good backup
                // rather than reporting "no sessions" and risking an overwrite.
                if let Ok(bak) = Self::bak_path() {
                    if let Ok(btxt) = fs::read_to_string(&bak) {
                        if let Ok(file) = serde_json::from_str::<SessionsFile>(&btxt) {
                            return Ok(file);
                        }
                    }
                }
                Err(AppError::Other(format!("parse sessions.json: {e}")))
            }
        }
    }

    /// Write atomically: serialize to a temp file, back up the current good
    /// file, then rename the temp over the target. A crash mid-write can only
    /// damage the temp file, never the live sessions.json.
    fn write_file(file: &SessionsFile) -> AppResult<()> {
        let path = Self::path()?;
        let json = serde_json::to_string_pretty(file)
            .map_err(|e| AppError::Other(format!("serialize: {e}")))?;
        let tmp = Self::tmp_path()?;
        fs::write(&tmp, json.as_bytes())?;
        // Snapshot the previous (known-good, since save() loaded it) file before
        // replacing it, so a future corruption has something to recover from.
        if path.exists() {
            if let Ok(bak) = Self::bak_path() {
                let _ = fs::copy(&path, &bak);
            }
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn list(&self, project_root: &str) -> AppResult<Vec<PersistedSession>> {
        let _g = self.lock.lock().unwrap();
        let file = Self::load_file()?;
        Ok(file.by_project.get(project_root).cloned().unwrap_or_default())
    }

    pub fn save(&self, project_root: &str, sessions: Vec<PersistedSession>) -> AppResult<()> {
        let _g = self.lock.lock().unwrap();
        // If the existing file can't be read, abort rather than clobbering the
        // other projects' history with a blind overwrite.
        let mut file = Self::load_file()?;
        if sessions.is_empty() {
            file.by_project.remove(project_root);
        } else {
            file.by_project.insert(project_root.to_string(), sessions);
        }
        Self::write_file(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample(id: &str) -> PersistedSession {
        PersistedSession {
            id: id.into(),
            name: format!("name-{id}"),
            cwd: "/tmp".into(),
            created_at_ms: 1,
            scrollback: "output".into(),
            agent: None,
            claude_session_id: None,
            name_suggested: None,
            model: None,
        }
    }

    /// Exercises the real load/save code in an isolated data dir (via
    /// XDG_DATA_HOME, which dirs::data_local_dir respects) so the user's real
    /// sessions.json is never touched. One test fn on purpose: it mutates a
    /// process-global env var, so it must not race a sibling test.
    #[test]
    fn persistence_is_crash_safe() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir()
            .join(format!("ac-sessions-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = SessionsService::new();
        let proj = "/proj/a";

        // 1. Fresh install (no file): empty, and crucially NOT an error.
        assert!(svc.list(proj).unwrap().is_empty(), "fresh list should be empty");

        // 2. Save then read back; temp file must be renamed away (atomic write).
        svc.save(proj, vec![sample("s1"), sample("s2")]).unwrap();
        assert_eq!(svc.list(proj).unwrap().len(), 2);
        let path = SessionsService::path().unwrap();
        let bak = SessionsService::bak_path().unwrap();
        let tmp = SessionsService::tmp_path().unwrap();
        assert!(path.exists(), "main file should exist after save");
        assert!(!tmp.exists(), "atomic write must leave no .tmp behind");

        // 3. A later save snapshots the prior good file to .bak, and a save to a
        //    DIFFERENT project leaves this one's data intact.
        svc.save(proj, vec![sample("s1")]).unwrap();
        assert!(bak.exists(), "backup should exist after a second save");
        svc.save("/proj/b", vec![sample("b1")]).unwrap();
        assert_eq!(svc.list(proj).unwrap().len(), 1, "proj a untouched by proj b save");
        assert_eq!(svc.list("/proj/b").unwrap().len(), 1);

        // 4. CRITICAL: corrupt main file but a valid backup → recover from the
        //    backup, never silently report "no sessions".
        let good = std::fs::read_to_string(&bak).unwrap();
        std::fs::write(&path, "{ not valid json ").unwrap();
        std::fs::write(&bak, &good).unwrap();
        assert!(
            !svc.list(proj).unwrap().is_empty(),
            "corrupt main must recover from backup, not return empty"
        );

        // 5. CRITICAL: corrupt main AND no usable backup → ERROR (not empty), and
        //    save must ABORT rather than clobber unreadable history.
        std::fs::write(&path, "garbage").unwrap();
        std::fs::write(&bak, "also garbage").unwrap();
        assert!(svc.list(proj).is_err(), "unreadable history must error, never look empty");
        assert!(
            svc.save(proj, vec![sample("x")]).is_err(),
            "save must not overwrite history it could not read"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
