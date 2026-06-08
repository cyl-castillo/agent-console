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
            // Register the lightweight UserPromptSubmit observer on first run so
            // session-name suggestions / resume binding / activity / snapshots
            // work without the user having to flip the integration toggle.
            if let Err(e) = state.hooks.ensure_autoinstalled() {
                eprintln!("hooks: auto-install failed: {e}");
            }
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
            commands::git::git_recent_messages,
            commands::git::git_head_message,
            commands::git::git_amend_commit,
            commands::git::git_file_log,
            commands::git::git_branches,
            commands::git::git_checkout_branch,
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
            commands::usage::session_usage,
            commands::advisor::advisor_analyze,
            commands::advisor::advisor_create_skill,
            commands::learning::learning_reflect,
            commands::learning::activity_list,
            commands::learning::learning_create_skill,
            commands::learning::learning_save_memory,
            commands::vault::vault_list,
            commands::vault::vault_upsert,
            commands::vault::vault_delete,
            commands::vault::vault_get_value,
            commands::context::context_status,
            commands::context::context_read_md,
            commands::context::context_write_md,
            commands::context::context_open_md_externally,
            commands::context::context_generate_starter,
            commands::context::memory_list,
            commands::context::memory_read,
            commands::context::memory_delete,
            commands::palette::palette_index_files,
            commands::feedback::feedback_dev_enabled,
            commands::feedback::feedback_context,
            commands::feedback::feedback_submit,
            commands::plugins::plugins_list_installed,
            commands::plugins::plugins_list_available,
            commands::plugins::plugins_install,
            commands::mcp::mcp_list,
            commands::mcp::mcp_add,
            commands::mcp::mcp_remove,
            commands::roundtable::roundtable_start,
            commands::roundtable::roundtable_pause,
            commands::roundtable::roundtable_resume,
            commands::roundtable::roundtable_inject,
            commands::roundtable::roundtable_continue,
            commands::roundtable::roundtable_stop,
            commands::roundtable::roundtable_discard,
            commands::roundtable::roundtable_list_rooms,
            commands::roundtable::roundtable_get_room,
            commands::roundtable::roundtable_delete_room,
            commands::roundtable::roundtable_resume_room,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Test-only support shared across modules. Placed at the end of the crate so it
/// doesn't trip clippy's `items_after_test_module` (a cfg(test) module followed
/// by real items).
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Mutex, MutexGuard};

    /// Serializes tests that mutate the process-global `XDG_DATA_HOME` (the
    /// persistence crash-safety tests in sessions/roundtable/activity all isolate
    /// their data dir that way). Without this they race: one test's set_var swaps
    /// the dir out from under another mid-assertion. Hold the guard for the whole
    /// test body. Poison is recovered — a panic in one test must not cascade.
    pub static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }
}
