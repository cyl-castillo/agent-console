mod commands;
mod error;
mod services;
mod state;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
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
            commands::chat::chat_send,
            commands::chat::chat_reset,
            commands::snapshot::snapshot_restore,
            commands::snapshot::snapshot_delete,
            commands::permission::perm_respond,
            commands::permission::perm_set_approve_all,
            commands::projects::projects_recent,
            commands::projects::projects_last,
            commands::projects::projects_forget,
            commands::projects::projects_remember,
            commands::task::task_save,
            commands::task::task_list,
            commands::task::task_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
