use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::services::tts_service;
use crate::services::voice_service::{self, VoiceStatus};
use crate::state::AppState;

/// Enable voice: download the Whisper model on first use (emitting
/// `voice://model-progress`), load it, then fetch Piper and its voice on the
/// same progress bar. Piper is best-effort — the platform TTS covers for it.
#[tauri::command]
pub async fn voice_enable(app: AppHandle, state: State<'_, AppState>) -> AppResult<VoiceStatus> {
    let path = voice_service::ensure_model(&app).await?;
    let svc = state.voice.clone();
    // Loading ~200 MB of weights blocks for a moment — keep it off the runtime.
    tokio::task::spawn_blocking(move || svc.load_model(&path))
        .await
        .map_err(|e| AppError::Other(format!("voice load task panicked: {e}")))??;
    if let Err(e) = tts_service::ensure_piper(&app).await {
        tracing::warn!("voice: Piper unavailable, using the platform TTS: {e}");
    }
    Ok(state.voice.status())
}

/// Drop the model and stop any capture; voice memory is fully released.
#[tauri::command(async)]
pub fn voice_disable(state: State<'_, AppState>) -> AppResult<VoiceStatus> {
    state.tts.stop();
    state.voice.disable();
    Ok(state.voice.status())
}

#[tauri::command]
pub fn voice_status(state: State<'_, AppState>) -> AppResult<VoiceStatus> {
    Ok(state.voice.status())
}

/// Start push-to-talk capture (mic opens; samples accumulate until stop).
/// Cuts off anything the console is saying: the user wants the floor.
#[tauri::command]
pub async fn voice_ptt_start(state: State<'_, AppState>) -> AppResult<()> {
    state.tts.stop();
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

/// Speak text aloud and return once it has been heard. `false` = cut short
/// (barge-in or a newer announcement), so a caller meaning to listen next
/// knows the question never landed.
#[tauri::command]
pub async fn voice_speak(text: String, state: State<'_, AppState>) -> AppResult<bool> {
    let tts = state.tts.clone();
    tokio::task::spawn_blocking(move || tts.speak(&text))
        .await
        .map_err(|e| AppError::Other(format!("voice speak task panicked: {e}")))?
}

/// Silence the console mid-sentence.
#[tauri::command(async)]
pub fn voice_stop_speaking(state: State<'_, AppState>) -> AppResult<()> {
    state.tts.stop();
    Ok(())
}

/// Play a short cue: "listen" | "done" | "ack" | "error".
#[tauri::command]
pub async fn voice_earcon(kind: String, state: State<'_, AppState>) -> AppResult<()> {
    let tts = state.tts.clone();
    tokio::task::spawn_blocking(move || tts.earcon(&kind))
        .await
        .map_err(|e| AppError::Other(format!("voice earcon task panicked: {e}")))?
}

/// Open the mic for a fixed window and return the transcript (used for spoken
/// yes/no confirmations after an announcement).
#[tauri::command]
pub async fn voice_listen(state: State<'_, AppState>, seconds: f32) -> AppResult<String> {
    let svc = state.voice.clone();
    tokio::task::spawn_blocking(move || svc.listen_window(seconds))
        .await
        .map_err(|e| AppError::Other(format!("voice listen task panicked: {e}")))?
}
