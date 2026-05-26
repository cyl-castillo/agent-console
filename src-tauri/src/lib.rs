mod commands;
mod error;
mod services;
mod state;

use crate::state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Start the events.jsonl watcher as soon as the app is up so
            // hook events arriving from any terminal session are observed.
            let state = app.state::<AppState>();
            state.hooks.start_watcher(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::project::open_project,
            commands::project::read_tree,
            commands::project::current_project,
            commands::project::read_file_text,
            commands::project::workspace_context,
            commands::terminal::term_spawn,
            commands::terminal::term_write,
            commands::terminal::term_resize,
            commands::terminal::term_kill,
            commands::git::git_status,
            commands::git::git_diff_file,
            commands::git::git_revert_file,
            commands::git::git_stage_file,
            commands::git::git_unstage_file,
            commands::git::git_commit,
            commands::git::git_file_log,
            commands::snapshot::snapshot_restore,
            commands::snapshot::snapshot_delete,
            commands::projects::projects_recent,
            commands::projects::projects_last,
            commands::projects::projects_forget,
            commands::projects::projects_remember,
            commands::skills::skill_list,
            commands::skills::skill_read,
            commands::hooks::hooks_status,
            commands::hooks::hooks_install,
            commands::hooks::hooks_uninstall,
            commands::hooks::hooks_start_watcher,
            commands::hooks::approval_respond,
            commands::permissions::permissions_snapshot,
            commands::permissions::permissions_add,
            commands::permissions::permissions_remove,
            commands::sessions::sessions_list,
            commands::sessions::sessions_save,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
