//! Local voice input: microphone capture (cpal) + on-device speech-to-text
//! (whisper.cpp via whisper-rs). Audio never leaves the machine — the model
//! file is downloaded once from Hugging Face and inference runs on CPU threads.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use parking_lot::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::error::{AppError, AppResult};

/// Whisper expects 16 kHz mono f32 PCM.
const WHISPER_SAMPLE_RATE: u32 = 16_000;
/// Releases shorter than this are accidental taps; whisper hallucinates on them.
const MIN_AUDIO_SECS: f32 = 0.3;
/// Emit a download-progress event at most every this many bytes.
const PROGRESS_EVERY_BYTES: u64 = 2 * 1024 * 1024;

/// Quantized small model: best Spanish-quality/latency tradeoff under ~200 MB.
fn model_file() -> String {
    std::env::var("AGENT_CONSOLE_VOICE_MODEL").unwrap_or_else(|_| "ggml-small-q5_1.bin".into())
}

fn model_language() -> String {
    std::env::var("AGENT_CONSOLE_VOICE_LANG").unwrap_or_else(|_| "es".into())
}

fn model_path() -> AppResult<PathBuf> {
    let base =
        dirs::data_dir().ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?;
    Ok(base.join("agent-console").join("models").join(model_file()))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    pub enabled: bool,
    pub capturing: bool,
    pub model_present: bool,
    pub model_file: String,
    pub language: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ModelProgress {
    downloaded: u64,
    total: Option<u64>,
}

struct Capture {
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
    join: JoinHandle<()>,
}

/// Cheaply cloneable (Arc'd) so commands can move it into spawn_blocking.
#[derive(Clone, Default)]
pub struct VoiceService {
    ctx: Arc<Mutex<Option<Arc<WhisperContext>>>>,
    capture: Arc<Mutex<Option<Capture>>>,
}

/// Download the model file if missing (atomic: `.part` + rename), emitting
/// `voice://model-progress` along the way. Returns the model path.
pub async fn ensure_model(app: &AppHandle) -> AppResult<PathBuf> {
    let path = model_path()?;
    if path.exists() {
        return Ok(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        model_file()
    );
    let mut resp = reqwest::get(&url)
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| AppError::Other(format!("model download failed: {e}")))?;
    let total = resp.content_length();
    let part = path.with_file_name(format!("{}.part", model_file()));
    let mut file = std::fs::File::create(&part)?;
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| AppError::Other(format!("model download interrupted: {e}")))?
    {
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        if downloaded - last_emit >= PROGRESS_EVERY_BYTES {
            last_emit = downloaded;
            let _ = app.emit("voice://model-progress", ModelProgress { downloaded, total });
        }
    }
    file.sync_all()?;
    drop(file);
    std::fs::rename(&part, &path)?;
    let _ = app.emit("voice://model-progress", ModelProgress { downloaded, total });
    Ok(path)
}

impl VoiceService {
    pub fn status(&self) -> VoiceStatus {
        VoiceStatus {
            enabled: self.ctx.lock().is_some(),
            capturing: self.capture.lock().is_some(),
            model_present: model_path().map(|p| p.exists()).unwrap_or(false),
            model_file: model_file(),
            language: model_language(),
        }
    }

    /// Blocking: load whisper weights into memory (no-op when already loaded).
    pub fn load_model(&self, path: &Path) -> AppResult<()> {
        let mut guard = self.ctx.lock();
        if guard.is_some() {
            return Ok(());
        }
        let path_str = path
            .to_str()
            .ok_or_else(|| AppError::Other("model path is not utf-8".into()))?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .map_err(|e| AppError::Other(format!("failed to load whisper model: {e}")))?;
        *guard = Some(Arc::new(ctx));
        Ok(())
    }

    /// Drop the model and stop any in-flight capture; frees all voice memory.
    pub fn disable(&self) {
        if let Some(cap) = self.capture.lock().take() {
            cap.stop.store(true, Ordering::Relaxed);
            let _ = cap.join.join();
        }
        *self.ctx.lock() = None;
    }

    /// Blocking: open the default input device and start accumulating samples.
    pub fn start_capture(&self) -> AppResult<()> {
        if self.ctx.lock().is_none() {
            return Err(AppError::Other("voice mode is not enabled".into()));
        }
        let mut slot = self.capture.lock();
        if slot.is_some() {
            return Ok(()); // already listening (key-repeat safety)
        }
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(u32, u16), String>>();
        let join = {
            let stop = stop.clone();
            let samples = samples.clone();
            std::thread::spawn(move || capture_thread(stop, samples, ready_tx))
        };
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok((sample_rate, channels))) => {
                *slot = Some(Capture { stop, samples, sample_rate, channels, join });
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = join.join();
                Err(AppError::Other(e))
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err(AppError::Other("microphone init timed out".into()))
            }
        }
    }

    /// Blocking: stop the capture, run whisper, return the transcript (empty
    /// when the recording was too short or silent).
    pub fn stop_and_transcribe(&self) -> AppResult<String> {
        let cap = self
            .capture
            .lock()
            .take()
            .ok_or_else(|| AppError::Other("not capturing".into()))?;
        cap.stop.store(true, Ordering::Relaxed);
        let _ = cap.join.join();
        let raw = std::mem::take(&mut *cap.samples.lock());
        let ctx = self
            .ctx
            .lock()
            .clone()
            .ok_or_else(|| AppError::Other("voice mode is not enabled".into()))?;

        let mono = to_mono(&raw, cap.channels);
        let audio = resample(&mono, cap.sample_rate, WHISPER_SAMPLE_RATE);
        if (audio.len() as f32) < MIN_AUDIO_SECS * WHISPER_SAMPLE_RATE as f32 {
            return Ok(String::new());
        }
        transcribe(&ctx, &audio)
    }

    /// Blocking: open the mic for a fixed window (voice confirmations like
    /// "sí"/"no" after an announcement), then transcribe what was heard.
    pub fn listen_window(&self, seconds: f32) -> AppResult<String> {
        if self.capture.lock().is_some() {
            return Err(AppError::Other("already capturing".into()));
        }
        let secs = if seconds.is_finite() { seconds.clamp(1.0, 10.0) } else { 4.0 };
        self.start_capture()?;
        std::thread::sleep(Duration::from_secs_f32(secs));
        self.stop_and_transcribe()
    }
}

/// Speak `text` through speech-dispatcher (`spd-say -w`), in the same language
/// voice input uses. Blocking until the utterance finishes so callers can
/// sequence speak → listen without overlap.
pub fn speak(text: &str) -> AppResult<()> {
    let lang = model_language();
    let status = std::process::Command::new("spd-say")
        .args(["-w", "-l", &lang])
        .arg(text)
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::Other("spd-say not found — install speech-dispatcher".into())
            } else {
                AppError::Other(format!("tts: {e}"))
            }
        })?;
    if !status.success() {
        return Err(AppError::Other(format!("spd-say exited with {status}")));
    }
    Ok(())
}

fn transcribe(ctx: &WhisperContext, audio: &[f32]) -> AppResult<String> {
    let mut state = ctx
        .create_state()
        .map_err(|e| AppError::Other(format!("whisper state: {e}")))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    let lang = model_language();
    params.set_language(Some(&lang));
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4))
        .unwrap_or(2);
    params.set_n_threads(threads as i32);
    state
        .full(params, audio)
        .map_err(|e| AppError::Other(format!("whisper inference: {e}")))?;
    let mut text = String::new();
    for i in 0..state.full_n_segments() {
        let Some(seg) = state.get_segment(i) else { continue };
        let Ok(piece) = seg.to_str() else { continue };
        let piece = piece.trim();
        // Whisper marks non-speech as bracketed annotations ("[BLANK_AUDIO]",
        // "(música)") — noise for a command composer.
        if piece.is_empty() || piece.starts_with('[') || piece.starts_with('(') {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(piece);
    }
    Ok(text)
}

/// Owns the cpal stream for one push-to-talk hold. cpal streams are !Send, so
/// the stream lives and dies on this thread; `ready` reports the device's
/// native (rate, channels) once audio is flowing, or the open error.
fn capture_thread(
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    ready: mpsc::Sender<Result<(u32, u16), String>>,
) {
    let host = cpal::default_host();
    let Some(device) = host.default_input_device() else {
        let _ = ready.send(Err("no microphone found".into()));
        return;
    };
    let supported = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(format!("microphone config: {e}")));
            return;
        }
    };
    let sample_rate = supported.sample_rate();
    let channels = supported.channels();
    let config = supported.config();

    let built = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            let sink = samples.clone();
            device.build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    sink.lock().extend_from_slice(data);
                },
                |e| eprintln!("voice: stream error: {e}"),
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let sink = samples.clone();
            device.build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    sink.lock()
                        .extend(data.iter().map(|&v| v as f32 / 32768.0));
                },
                |e| eprintln!("voice: stream error: {e}"),
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let sink = samples.clone();
            device.build_input_stream(
                config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    sink.lock()
                        .extend(data.iter().map(|&v| (v as f32 - 32768.0) / 32768.0));
                },
                |e| eprintln!("voice: stream error: {e}"),
                None,
            )
        }
        other => {
            let _ = ready.send(Err(format!("unsupported sample format: {other:?}")));
            return;
        }
    };
    let stream = match built {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(format!("microphone open failed: {e}")));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("microphone start failed: {e}")));
        return;
    }
    let _ = ready.send(Ok((sample_rate, channels)));
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(stream);
}

/// Average interleaved channels down to mono.
fn to_mono(input: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return input.to_vec();
    }
    input
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Linear-interpolation resampler — plenty for speech into whisper.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input[idx];
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_mono_averages_stereo_frames() {
        let stereo = [0.0, 1.0, 0.5, 0.5, -1.0, 1.0];
        assert_eq!(to_mono(&stereo, 2), vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn to_mono_passes_mono_through() {
        let mono = [0.1, 0.2, 0.3];
        assert_eq!(to_mono(&mono, 1), mono.to_vec());
    }

    #[test]
    fn resample_halves_length_when_downsampling_2x() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = resample(&input, 32_000, 16_000);
        assert_eq!(out.len(), 50);
        // Linear interp of a linear ramp stays on the ramp.
        assert!((out[10] - 20.0).abs() < 1e-3);
    }

    #[test]
    fn resample_same_rate_is_identity() {
        let input = vec![0.1f32, 0.2, 0.3];
        assert_eq!(resample(&input, 16_000, 16_000), input);
    }
}
