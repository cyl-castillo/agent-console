use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use parking_lot::Mutex;

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
    /// Isolated worktree this session runs in (path + branch + base). Absent =
    /// the session runs directly in the project checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<crate::services::worktree_service::WorktreeInfo>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionsFile {
    #[serde(default)]
    by_project: HashMap<String, Vec<PersistedSession>>,
}

/// Produce a stable lookup key for a project path so the same folder always
/// maps to the same history bucket. History is indexed by the project path the
/// folder picker handed back; on Windows that picker can return the SAME
/// directory with a different drive-letter case or slash direction across app
/// versions/reinstalls. An exact-string HashMap key then misses and the user's
/// history looks erased even though it's still on disk under the old spelling.
/// We collapse those Windows-only variants (lowercase + forward slashes + no
/// trailing slash). POSIX paths are case-sensitive, so there we only trim a
/// trailing slash and never touch case.
fn normalize_key(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let is_windows_path = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    if is_windows_path {
        let lowered = raw.to_lowercase().replace('\\', "/");
        let trimmed = lowered.trim_end_matches('/');
        // Keep a bare drive root as "c:/" rather than collapsing it to "c:".
        if trimmed.len() == 2 {
            format!("{trimmed}/")
        } else {
            trimmed.to_string()
        }
    } else {
        let trimmed = raw.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

pub struct SessionsService {
    lock: Mutex<()>,
}

impl SessionsService {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
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
        // On Windows, antivirus/indexers briefly lock recently-written files
        // and the rename fails transiently; without a retry, every persist in
        // that window fails and (before the failure toast existed) session
        // history silently stopped saving — issue #72's likeliest mechanism.
        let mut last = None;
        for attempt in 0..3 {
            match fs::rename(&tmp, &path) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last = Some(e);
                    if attempt < 2 {
                        std::thread::sleep(std::time::Duration::from_millis(80));
                    }
                }
            }
        }
        Err(last.expect("loop ran").into())
    }

    pub fn list(&self, project_root: &str) -> AppResult<Vec<PersistedSession>> {
        let _g = self.lock.lock();
        let file = Self::load_file()?;
        if let Some(v) = file.by_project.get(project_root) {
            return Ok(v.clone());
        }
        // No exact match: the path may be stored under an equivalent but
        // differently-spelled key (Windows drive-letter case / slash direction
        // drifted across versions). Recover by matching on the normalized key so
        // history isn't reported as lost. The next save() collapses it back to
        // the current spelling.
        let target = normalize_key(project_root);
        for (k, v) in &file.by_project {
            if normalize_key(k) == target {
                return Ok(v.clone());
            }
        }
        Ok(Vec::new())
    }

    pub fn save(&self, project_root: &str, mut sessions: Vec<PersistedSession>) -> AppResult<()> {
        // Scrollback is unbounded in memory (xterm keeps its own cap), but on
        // disk it multiplies: N sessions × M projects, cloned on every list().
        // Keep only the newest tail per session — enough to repaint a resumed
        // terminal, without letting sessions.json grow to tens of MB.
        for s in &mut sessions {
            cap_scrollback(&mut s.scrollback);
        }
        let _g = self.lock.lock();
        // If the existing file can't be read, abort rather than clobbering the
        // other projects' history with a blind overwrite.
        let mut file = Self::load_file()?;
        // Absorb any equivalent-but-differently-spelled keys for this same folder
        // into the current spelling, so old orphaned entries are migrated rather
        // than left as stale duplicates. Keep the exact current key; drop only
        // the equivalent variants.
        let target = normalize_key(project_root);
        file.by_project
            .retain(|k, _| k == project_root || normalize_key(k) != target);
        if sessions.is_empty() {
            file.by_project.remove(project_root);
        } else {
            file.by_project.insert(project_root.to_string(), sessions);
        }
        Self::write_file(&file)
    }
}

/// Persisted-scrollback cap: 100 KB is several screens of history — plenty to
/// repaint a resumed session. Trimming keeps the TAIL (newest output) and cuts
/// at a char boundary so the stored string stays valid UTF-8.
const SCROLLBACK_MAX_BYTES: usize = 100 * 1024;

fn cap_scrollback(s: &mut String) {
    if s.len() <= SCROLLBACK_MAX_BYTES {
        return;
    }
    let mut start = s.len() - SCROLLBACK_MAX_BYTES;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s.replace_range(..start, "");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn scrollback_cap_keeps_newest_tail_at_char_boundary() {
        let mut small = "hola".to_string();
        cap_scrollback(&mut small);
        assert_eq!(small, "hola", "under the cap: untouched");

        // Multibyte content straddling the cut point must not panic and must
        // keep the tail.
        let mut big = "ñ".repeat(SCROLLBACK_MAX_BYTES); // 2 bytes each
        big.push_str("FIN");
        cap_scrollback(&mut big);
        assert!(big.len() <= SCROLLBACK_MAX_BYTES);
        assert!(big.ends_with("FIN"), "newest output survives");
        assert!(big.starts_with('ñ'), "cut lands on a char boundary");
    }

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
            worktree: None,
        }
    }

    /// Exercises the real load/save code in an isolated data dir (via
    /// XDG_DATA_HOME, which dirs::data_local_dir respects) so the user's real
    /// sessions.json is never touched. One test fn on purpose: it mutates a
    /// process-global env var, so it must not race a sibling test.
    #[test]
    fn persistence_is_crash_safe() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-sessions-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = SessionsService::new();
        let proj = "/proj/a";

        // 1. Fresh install (no file): empty, and crucially NOT an error.
        assert!(
            svc.list(proj).unwrap().is_empty(),
            "fresh list should be empty"
        );

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
        assert_eq!(
            svc.list(proj).unwrap().len(),
            1,
            "proj a untouched by proj b save"
        );
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
        assert!(
            svc.list(proj).is_err(),
            "unreadable history must error, never look empty"
        );
        assert!(
            svc.save(proj, vec![sample("x")]).is_err(),
            "save must not overwrite history it could not read"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// normalize_key collapses the Windows-only path variants (drive-letter
    /// case, slash direction, trailing slash) that caused history to look lost
    /// after an update, while keeping case-sensitive POSIX paths distinct.
    #[test]
    fn normalize_key_collapses_windows_variants() {
        // Same Windows folder spelled four ways → one key.
        let canon = normalize_key("C:\\Users\\Foo\\Proj");
        assert_eq!(canon, normalize_key("c:/users/foo/proj"));
        assert_eq!(canon, normalize_key("C:/Users/Foo/Proj/"));
        assert_eq!(canon, normalize_key("c:\\users\\foo\\proj\\"));
        assert_eq!(canon, "c:/users/foo/proj");

        // A bare drive root stays a drive root.
        assert_eq!(normalize_key("C:\\"), "c:/");
        assert_eq!(normalize_key("c:/"), "c:/");

        // POSIX: trailing slash trimmed, but case is preserved (case-sensitive FS).
        assert_eq!(normalize_key("/proj/a/"), "/proj/a");
        assert_eq!(normalize_key("/proj/a"), "/proj/a");
        assert_ne!(normalize_key("/Proj"), normalize_key("/proj"));
        assert_eq!(normalize_key("/"), "/");
    }

    /// History saved under one spelling of a Windows path is recovered when the
    /// project is reopened under an equivalent spelling, and the next save
    /// collapses the old key into the new one instead of leaving a duplicate.
    #[test]
    fn recovers_history_across_windows_path_spellings() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-sessions-norm-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);

        let svc = SessionsService::new();
        let old_key = "C:\\Users\\Foo\\Proj";
        let new_key = "c:/users/foo/proj";

        // 1. History saved under the old spelling...
        svc.save(old_key, vec![sample("s1"), sample("s2")]).unwrap();

        // 2. ...is recovered when the folder is reopened under the new spelling.
        assert_eq!(
            svc.list(new_key).unwrap().len(),
            2,
            "history must be found under an equivalent Windows path spelling"
        );

        // 3. Saving under the new spelling collapses the old key — no duplicate
        //    bucket lingers, and the data lives under the current spelling.
        svc.save(new_key, vec![sample("s1")]).unwrap();
        let file = SessionsService::load_file().unwrap();
        assert_eq!(
            file.by_project.len(),
            1,
            "equivalent old key must be absorbed, not duplicated"
        );
        assert!(
            file.by_project.contains_key(new_key),
            "data should live under the current spelling after save"
        );
        assert_eq!(svc.list(old_key).unwrap().len(), 1, "old spelling still resolves");
        assert_eq!(svc.list(new_key).unwrap().len(), 1, "new spelling resolves");

        let _ = std::fs::remove_dir_all(&base);
    }
}
