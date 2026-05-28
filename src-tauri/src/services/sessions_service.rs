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
    /// Claude Code session id captured from the UserPromptSubmit hook; used to
    /// auto-resume a Claude conversation when the user reactivates this terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    /// True once the rename suggestion has been offered for this session, so
    /// we don't re-suggest on every subsequent prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_suggested: Option<bool>,
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

    fn path() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir.join("sessions.json"))
    }

    fn load_file() -> SessionsFile {
        let Ok(path) = Self::path() else { return SessionsFile::default() };
        let Ok(txt) = fs::read_to_string(&path) else { return SessionsFile::default() };
        serde_json::from_str(&txt).unwrap_or_default()
    }

    fn write_file(file: &SessionsFile) -> AppResult<()> {
        let path = Self::path()?;
        let json = serde_json::to_string_pretty(file)
            .map_err(|e| AppError::Other(format!("serialize: {e}")))?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn list(&self, project_root: &str) -> Vec<PersistedSession> {
        let _g = self.lock.lock().unwrap();
        let file = Self::load_file();
        file.by_project.get(project_root).cloned().unwrap_or_default()
    }

    pub fn save(&self, project_root: &str, sessions: Vec<PersistedSession>) -> AppResult<()> {
        let _g = self.lock.lock().unwrap();
        let mut file = Self::load_file();
        if sessions.is_empty() {
            file.by_project.remove(project_root);
        } else {
            file.by_project.insert(project_root.to_string(), sessions);
        }
        Self::write_file(&file)
    }
}
