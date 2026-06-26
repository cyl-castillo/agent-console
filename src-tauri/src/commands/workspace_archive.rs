use std::fs;
use std::path::Path;

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::workspace_archive::{
    self, ExportOptions, ImportDecisions, ImportManifest, ImportResult,
};
use crate::state::AppState;

/// What the export wrote, so the UI can confirm ("exported 12 sessions, 3 rooms…
/// to <file>") without re-reading the file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub bytes: u64,
    pub sessions: usize,
    pub rooms: usize,
    pub schedules: usize,
    pub skills: usize,
    pub memory: usize,
}

/// Build the archive for `project_root` per `options` and write it to
/// `dest_path` (a location the user picked via the save dialog). The file is the
/// single source of truth that another user imports.
#[tauri::command]
pub fn export_work(
    project_root: String,
    options: ExportOptions,
    dest_path: String,
    state: State<'_, AppState>,
) -> AppResult<ExportResult> {
    let archive = workspace_archive::build_archive(&state, &project_root, options)?;
    let json = workspace_archive::to_json(&archive)?;

    let dest = Path::new(&dest_path);
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::Other(format!("create export dir: {e}")))?;
        }
    }
    fs::write(dest, json.as_bytes())
        .map_err(|e| AppError::Other(format!("write export file: {e}")))?;

    let b = &archive.blocks;
    let learning = b.learning.as_ref();
    Ok(ExportResult {
        path: dest_path,
        bytes: json.len() as u64,
        sessions: b.sessions.as_ref().map(|v| v.len()).unwrap_or(0),
        rooms: b.rooms.as_ref().map(|v| v.len()).unwrap_or(0),
        schedules: b.schedules.as_ref().map(|v| v.len()).unwrap_or(0),
        skills: learning.map(|l| l.skills.len()).unwrap_or(0),
        memory: learning.map(|l| l.memory.len()).unwrap_or(0),
    })
}

/// Read an archive file, validate it, and preview what importing it into
/// `project_root` would do (counts + collisions per block). No mutation.
#[tauri::command]
pub fn import_work_preview(
    project_root: String,
    src_path: String,
    state: State<'_, AppState>,
) -> AppResult<ImportManifest> {
    let json = fs::read_to_string(Path::new(&src_path))
        .map_err(|e| AppError::Other(format!("read import file: {e}")))?;
    let archive = workspace_archive::parse_archive(&json)?;
    workspace_archive::build_manifest(&state, &project_root, &archive)
}

/// Apply an archive file to `project_root` with the user's per-block decisions,
/// re-keying everything to the destination project.
#[tauri::command]
pub fn import_work_apply(
    project_root: String,
    src_path: String,
    decisions: ImportDecisions,
    state: State<'_, AppState>,
) -> AppResult<ImportResult> {
    let json = fs::read_to_string(Path::new(&src_path))
        .map_err(|e| AppError::Other(format!("read import file: {e}")))?;
    let archive = workspace_archive::parse_archive(&json)?;
    workspace_archive::apply_archive(&state, &project_root, &archive, decisions)
}
