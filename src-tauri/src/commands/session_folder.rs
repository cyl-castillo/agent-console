//! Per-session folders: a session runs in a folder other than the project
//! checkout, chosen with the native picker. See `linked_folders_service`.

use std::path::PathBuf;

use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::error::{AppError, AppResult};
use crate::services::linked_folders_service::{self, LinkedFolder};
use crate::state::AppState;

fn project_root(state: &AppState) -> AppResult<PathBuf> {
    state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::InvalidArgument("no project open".into()))
}

/// Open the native folder picker and link the chosen folder to the open
/// project. This is the only way a folder becomes one the git/file commands
/// will follow: the path comes from the OS dialog, never from the webview.
/// `None` when the user cancels.
#[tauri::command(async)]
pub fn folder_pick(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<LinkedFolder>> {
    let root = project_root(&state)?;
    let start = root
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Folder for this session")
        .set_directory(&start)
        .blocking_pick_folder()
    else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| AppError::Other(format!("picked folder: {e}")))?;
    linked_folders_service::link(&root.to_string_lossy(), &path).map(Some)
}

/// The open project's linked folders, most recently picked first.
#[tauri::command(async)]
pub fn linked_folders_list(state: State<'_, AppState>) -> AppResult<Vec<LinkedFolder>> {
    let root = project_root(&state)?;
    linked_folders_service::list(&root.to_string_lossy())
}

/// Take a folder off the open project's list. Sessions already running there
/// keep their terminal; git, files and Proof fall back to the project root
/// for them, since the folder is no longer authorized. Returns whether the
/// folder was on the list.
#[tauri::command(async)]
pub fn linked_folder_unlink(path: String, state: State<'_, AppState>) -> AppResult<bool> {
    let root = project_root(&state)?;
    linked_folders_service::unlink(&root.to_string_lossy(), &PathBuf::from(path))
}

/// The ledger a session running in `cwd` writes its evidence to — the same
/// rule the hook events are filed by, so the Proof panel shows exactly what
/// the session recorded.
#[tauri::command(async)]
pub fn ledger_root_for(cwd: String, state: State<'_, AppState>) -> AppResult<String> {
    let open = state
        .inner
        .lock()
        .project
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string());
    Ok(crate::services::hooks_service::ledger_root_for_cwd(
        &cwd,
        open.as_deref(),
    ))
}
