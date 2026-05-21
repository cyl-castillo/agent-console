use std::io::Read;
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::project_manager::{self, FileNode, Project, WorkspaceContext};
use crate::state::AppState;

const FILE_READ_LIMIT: u64 = 1024 * 1024; // 1 MB

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub content: String,
    pub is_binary: bool,
    pub size_bytes: u64,
    pub truncated: bool,
}

#[tauri::command]
pub fn open_project(path: String, state: State<'_, AppState>) -> AppResult<Project> {
    let path_buf = PathBuf::from(&path);
    let project = project_manager::open_project(&path_buf)?;
    let mut s = state.inner.lock().unwrap();
    s.project = Some(project.clone());
    // Record in recents — best effort, never fails the open.
    let _ = crate::services::projects_service::remember(&path_buf);
    Ok(project)
}

#[tauri::command]
pub fn read_tree(path: String, depth: Option<usize>) -> AppResult<FileNode> {
    let depth = depth.unwrap_or(3);
    project_manager::read_tree(&PathBuf::from(path), depth)
}

#[tauri::command]
pub fn current_project(state: State<'_, AppState>) -> Option<Project> {
    state.inner.lock().unwrap().project.clone()
}

#[tauri::command]
pub fn workspace_context(state: State<'_, AppState>) -> AppResult<WorkspaceContext> {
    let root = state.inner.lock().unwrap().project.as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))?;
    project_manager::workspace_context(&root)
}

/// Read a file for the Preview tab. Truncates to 1 MB and detects binaries
/// (null byte in the first 8 KB) so we never blast the webview with garbage.
#[tauri::command]
pub fn read_file_text(path: String) -> AppResult<FileContent> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(AppError::NotFound(p.display().to_string()));
    }
    if !p.is_file() {
        return Err(AppError::InvalidArgument(format!("not a file: {path}")));
    }
    let metadata = std::fs::metadata(&p)?;
    let size_bytes = metadata.len();
    let truncated = size_bytes > FILE_READ_LIMIT;
    let read_size = size_bytes.min(FILE_READ_LIMIT) as usize;

    let mut file = std::fs::File::open(&p)?;
    let mut buf = vec![0u8; read_size];
    file.read_exact(&mut buf)?;

    let probe = &buf[..buf.len().min(8192)];
    if probe.contains(&0) {
        return Ok(FileContent { content: String::new(), is_binary: true, size_bytes, truncated });
    }
    match String::from_utf8(buf) {
        Ok(content) => Ok(FileContent { content, is_binary: false, size_bytes, truncated }),
        Err(_) => Ok(FileContent { content: String::new(), is_binary: true, size_bytes, truncated }),
    }
}
