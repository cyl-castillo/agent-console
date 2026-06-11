use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::services::voice_service::{self, VoiceStatus};
use crate::state::AppState;

/// Enable voice input: download the Whisper model on first use (emitting
/// `voice://model-progress`), then load it into memory.
#[tauri::command]
pub async fn voice_enable(app: AppHandle, state: State<'_, AppState>) -> AppResult<VoiceStatus> {
    let path = voice_service::ensure_model(&app).await?;
    let svc = state.voice.clone();
    // Loading ~200 MB of weights blocks for a moment — keep it off the runtime.
    tokio::task::spawn_blocking(move || svc.load_model(&path))
        .await
        .map_err(|e| AppError::Other(format!("voice load task panicked: {e}")))??;
    Ok(state.voice.status())
}

/// Drop the model and stop any capture; voice memory is fully released.
#[tauri::command]
pub fn voice_disable(state: State<'_, AppState>) -> AppResult<VoiceStatus> {
    state.voice.disable();
    Ok(state.voice.status())
}

#[tauri::command]
pub fn voice_status(state: State<'_, AppState>) -> AppResult<VoiceStatus> {
    Ok(state.voice.status())
}

/// Start push-to-talk capture (mic opens; samples accumulate until stop).
#[tauri::command]
pub async fn voice_ptt_start(state: State<'_, AppState>) -> AppResult<()> {
    let svc = state.voice.clone();
    // Opening the input device can block for a moment on some backends.
    tokio::task::spawn_blocking(move || svc.start_capture())
        .await
        .map_err(|e| AppError::Other(format!("voice capture task panicked: {e}")))?
}

/// Stop capture and transcribe what was recorded. Returns the transcript
/// (empty string when the recording was too short or silent).
#[tauri::command]
pub async fn voice_ptt_stop(state: State<'_, AppState>) -> AppResult<String> {
    let svc = state.voice.clone();
    tokio::task::spawn_blocking(move || svc.stop_and_transcribe())
        .await
        .map_err(|e| AppError::Other(format!("voice transcribe task panicked: {e}")))?
}
