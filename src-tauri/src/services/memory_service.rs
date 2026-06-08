use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::context_service::memory_dir_for;

const MEMORY_INDEX: &str = "MEMORY.md";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub name: String,
    pub kind: Option<String>,
    pub description: Option<String>,
    pub size_bytes: u64,
    pub modified_ms: i64,
    pub is_index: bool,
}

pub fn list(project_root: &Path) -> AppResult<Vec<MemoryEntry>> {
    let dir = memory_dir_for(project_root)?;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir)?.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") {
            continue;
        }
        let meta = entry.metadata().ok();
        let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_ms = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let (kind, description) = parse_frontmatter(&p);
        out.push(MemoryEntry {
            is_index: name == MEMORY_INDEX,
            name,
            kind,
            description,
            size_bytes,
            modified_ms,
        });
    }
    // Index first, then by mtime desc.
    out.sort_by(|a, b| match (a.is_index, b.is_index) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => b.modified_ms.cmp(&a.modified_ms),
    });
    Ok(out)
}

pub fn read(project_root: &Path, name: &str) -> AppResult<String> {
    let path = safe_path(project_root, name)?;
    if !path.exists() {
        return Err(AppError::NotFound(format!("memory '{name}'")));
    }
    Ok(fs::read_to_string(&path)?)
}

/// Create or overwrite a memory entry. Used by "learning mode" to materialize an
/// accepted suggestion into the project's memory dir, where Claude reads it on
/// future sessions. Same path-traversal guard as the other operations; the
/// memory dir is created on demand (a fresh project has none yet). MEMORY.md is
/// off-limits — it's the hand-curated index, not a place for generated entries.
pub fn write(project_root: &Path, name: &str, content: &str) -> AppResult<PathBuf> {
    if name == MEMORY_INDEX {
        return Err(AppError::InvalidArgument(
            "refusing to overwrite MEMORY.md (the memory index)".into(),
        ));
    }
    let dir = memory_dir_for(project_root)?;
    fs::create_dir_all(&dir)?;
    let path = safe_path(project_root, name)?;
    fs::write(&path, content)?;
    Ok(path)
}

pub fn delete(project_root: &Path, name: &str) -> AppResult<()> {
    if name == MEMORY_INDEX {
        return Err(AppError::InvalidArgument(
            "refusing to delete MEMORY.md (the memory index)".into(),
        ));
    }
    let path = safe_path(project_root, name)?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Resolve a memory entry by name, defending against path traversal.
/// Rules: must end in `.md`, must not contain path separators or `..`,
/// and the resolved path must live inside the project's memory dir.
fn safe_path(project_root: &Path, name: &str) -> AppResult<PathBuf> {
    if name.is_empty()
        || !name.ends_with(".md")
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Err(AppError::InvalidArgument(format!(
            "invalid memory name: {name}"
        )));
    }
    let dir = memory_dir_for(project_root)?;
    let path = dir.join(name);
    // Defense in depth: confirm canonical path is still inside the dir.
    let canon = path.canonicalize().unwrap_or(path.clone());
    let dir_canon = dir.canonicalize().unwrap_or(dir.clone());
    if !canon.starts_with(&dir_canon) {
        return Err(AppError::InvalidArgument(format!(
            "path escapes memory dir: {name}"
        )));
    }
    Ok(path)
}

fn parse_frontmatter(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(content) = fs::read_to_string(path) else {
        return (None, None);
    };
    if !content.starts_with("---") {
        return (None, None);
    }
    let after = &content[3..];
    let Some(end) = after.find("\n---") else {
        return (None, None);
    };
    let fm = &after[..end];
    let mut description: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut in_metadata = false;
    for line in fm.lines() {
        let raw = line;
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("description:") {
            let v = rest
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            if !v.is_empty() {
                description = Some(v);
            }
        } else if l.starts_with("metadata:") {
            in_metadata = true;
        } else if in_metadata && raw.starts_with("  ") {
            if let Some(rest) = l.strip_prefix("type:") {
                let v = rest
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_string();
                if !v.is_empty() {
                    kind = Some(v);
                }
            }
        } else if !raw.starts_with(' ') {
            // Left the metadata block.
            in_metadata = false;
        }
    }
    (kind, description)
}
