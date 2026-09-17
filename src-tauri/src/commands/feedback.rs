use tauri::State;

use crate::error::AppResult;
use crate::services::feedback_service::{self, FeedbackContext, FeedbackInput};
use crate::state::AppState;

#[tauri::command]
pub fn feedback_dev_enabled() -> bool {
    feedback_service::dev_enabled()
}

#[tauri::command]
pub fn feedback_context(state: State<'_, AppState>) -> FeedbackContext {
    let s = state.inner.lock();
    let project = s.project.as_ref();
    feedback_service::context(
        project.map(|p| p.root.as_path()),
        project.map(|p| p.name.as_str()),
    )
}

#[tauri::command]
pub fn feedback_submit(input: FeedbackInput, state: State<'_, AppState>) -> AppResult<String> {
    let ctx = {
        let s = state.inner.lock();
        let project = s.project.as_ref();
        feedback_service::context(
            project.map(|p| p.root.as_path()),
            project.map(|p| p.name.as_str()),
        )
    };
    // The bundle rides along collapsed: hooks state, store sizes and the log
    // tail are what every "it doesn't work" report ends up needing.
    let diagnostics = crate::services::diagnostics::render(&super::diagnostics::collect(&state));
    feedback_service::submit(input, &ctx, Some(&diagnostics))
}
