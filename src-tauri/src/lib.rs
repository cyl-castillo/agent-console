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
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Start the events.jsonl watcher as soon as the app is up so
            // hook events arriving from any terminal session are observed.
            let state = app.state::<AppState>();
            state.hooks.start_watcher(app.handle().clone());
            // Start the scheduler tick loop: reconciles firings missed while the
            // app was closed, then runs due jobs on a calm poll. Suggest-only —
            // every job runs through plan-mode `claude`.
            state.scheduler.start(app.handle().clone());
            // Register the lightweight UserPromptSubmit observer on first run so
            // session-name suggestions / resume binding / activity / snapshots
            // work without the user having to flip the integration toggle.
            if let Err(e) = state.hooks.ensure_autoinstalled() {
                eprintln!("hooks: auto-install failed: {e}");
            }
            // Codex twin (own marker; no-op when the codex CLI isn't installed).
            if let Err(e) = state.hooks.ensure_codex_autoinstalled() {
                eprintln!("hooks: codex auto-install failed: {e}");
            }
            // Turn-completed observer for both engines (own marker so it
            // reaches installs that predate it).
            if let Err(e) = state.hooks.ensure_stop_autoinstalled() {
                eprintln!("hooks: stop auto-install failed: {e}");
            }
            // Tool-result observer (Testigo turn evidence), same rollout
            // pattern as Stop.
            if let Err(e) = state.hooks.ensure_posttooluse_autoinstalled() {
                eprintln!("hooks: posttooluse auto-install failed: {e}");
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
            commands::terminal::term_save_paste_image,
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
            commands::worktree::worktree_create,
            commands::worktree::worktree_suggest_branch,
            commands::worktree::worktree_status,
            commands::worktree::worktree_merge,
            commands::worktree::worktree_discard,
            commands::worktree::worktree_list,
            commands::worktree::worktree_setup_get,
            commands::worktree::worktree_setup_set,
            commands::worktree::set_active_repo,
            commands::worktree::worktree_prune_orphans,
            commands::preflight::preflight_check,
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
            commands::learning::learning_curate,
            commands::learning::activity_list,
            commands::learning::learning_create_skill,
            commands::learning::learning_create_plugin,
            commands::learning::learning_save_memory,
            commands::learning::learning_apply_refactor,
            commands::learning::learning_apply_merge,
            commands::learning::learning_apply_archive,
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
            commands::jira::jira_status,
            commands::jira::jira_connect,
            commands::jira::jira_disconnect,
            commands::jira::jira_list_issues,
            commands::notes::notes_list,
            commands::notes::notes_save,
            commands::hooks::approvals_pending,
            commands::testigo::testigo_list,
            commands::testigo::testigo_verify,
            commands::testigo::testigo_link_case,
            commands::testigo::testigo_export,
            commands::testigo::testigo_export_preview,
            commands::testigo::testigo_public_key,
            commands::testigo::testigo_get_settings,
            commands::testigo::testigo_set_settings,
            commands::plugins::plugins_list_installed,
            commands::plugins::plugins_list_available,
            commands::plugins::plugins_install,
            commands::plugins::plugins_update,
            commands::plugins::plugins_update_marketplaces,
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
            commands::roundtable::roundtable_share,
            commands::roundtable::roundtable_sync,
            commands::roundtable::roundtable_list_rooms,
            commands::roundtable::roundtable_get_room,
            commands::roundtable::roundtable_delete_room,
            commands::roundtable::roundtable_resume_room,
            commands::scheduler::scheduler_list,
            commands::scheduler::scheduler_create,
            commands::scheduler::scheduler_update,
            commands::scheduler::scheduler_delete,
            commands::scheduler::scheduler_set_enabled,
            commands::scheduler::scheduler_history,
            commands::scheduler::scheduler_is_paused,
            commands::scheduler::scheduler_set_paused,
            commands::scheduler::scheduler_fire_event,
            commands::scheduler::scheduler_run_now,
            commands::voice::voice_enable,
            commands::voice::voice_disable,
            commands::voice::voice_status,
            commands::voice::voice_ptt_start,
            commands::voice::voice_ptt_stop,
            commands::voice::voice_speak,
            commands::voice::voice_listen,
            commands::workspace_archive::export_work,
            commands::workspace_archive::import_work_preview,
            commands::workspace_archive::import_work_apply,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Test-only support shared across modules. Placed at the end of the crate so it
/// doesn't trip clippy's `items_after_test_module` (a cfg(test) module followed
/// by real items).
#[cfg(test)]
pub(crate) mod test_support {
    use parking_lot::{Mutex, MutexGuard};

    /// Serializes tests that mutate the process-global `XDG_DATA_HOME` (the
    /// persistence crash-safety tests in sessions/roundtable/activity all isolate
    /// their data dir that way). Without this they race: one test's set_var swaps
    /// the dir out from under another mid-assertion. Hold the guard for the whole
    /// test body. parking_lot does not poison, so a panic in one test does not
    /// cascade into the others still waiting on this lock.
    pub static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock()
    }
}
