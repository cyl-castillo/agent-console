use tauri::State;

use crate::error::{AppError, AppResult};
use crate::services::advisor_service::{self, AnalysisResult};
use crate::state::AppState;

#[tauri::command]
pub async fn advisor_analyze(state: State<'_, AppState>) -> AppResult<AnalysisResult> {
    let root = {
        let s = state.inner.lock().unwrap();
        s.project
            .as_ref()
            .map(|p| p.root.clone())
            .ok_or_else(|| AppError::Other("no project open".into()))?
    };
    // Offload the blocking `claude -p` call so we don't stall the runtime.
    tokio::task::spawn_blocking(move || advisor_service::analyze(&root))
        .await
        .map_err(|e| AppError::Other(format!("analysis task panicked: {e}")))?
}

#[tauri::command]
pub fn advisor_create_skill(
    state: State<'_, AppState>,
    scope: String,
    name: String,
    skill_md_content: String,
) -> AppResult<String> {
    let root = state
        .inner
        .lock()
        .unwrap()
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| AppError::Other("no project open".into()))?;
    let path = advisor_service::create_skill(&root, &scope, &name, &skill_md_content)?;
    Ok(path.display().to_string())
}
