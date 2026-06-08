use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub last_opened_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecentsFile {
    #[serde(default)]
    entries: Vec<RecentProject>,
}

const MAX_RECENT: usize = 12;

fn config_dir() -> AppResult<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AppError::Other("no config dir".into()))?
        .join("agent-console");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn recents_path() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("recents.json"))
}

pub fn load() -> Vec<RecentProject> {
    let Ok(path) = recents_path() else {
        return Vec::new();
    };
    let Ok(txt) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let file: RecentsFile = serde_json::from_str(&txt).unwrap_or_default();
    // Drop entries whose folder no longer exists.
    file.entries
        .into_iter()
        .filter(|e| Path::new(&e.path).is_dir())
        .collect()
}

pub fn remember(path: &Path) -> AppResult<()> {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let entry = RecentProject {
        path: path.display().to_string(),
        name,
        last_opened_ms: now,
    };

    let mut entries = load();
    entries.retain(|e| e.path != entry.path);
    entries.insert(0, entry);
    if entries.len() > MAX_RECENT {
        entries.truncate(MAX_RECENT);
    }
    let file = RecentsFile { entries };
    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| AppError::Other(format!("serialize: {e}")))?;
    fs::write(recents_path()?, json)?;
    Ok(())
}

pub fn last() -> Option<RecentProject> {
    load().into_iter().next()
}

pub fn forget(path: &str) -> AppResult<()> {
    let mut entries = load();
    entries.retain(|e| e.path != path);
    let file = RecentsFile { entries };
    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| AppError::Other(format!("serialize: {e}")))?;
    fs::write(recents_path()?, json)?;
    Ok(())
}
