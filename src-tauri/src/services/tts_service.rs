//! Spoken output. Piper (neural, local, fetched on first use) when it is
//! installed; the platform's own TTS otherwise, so the console is never mute.
//!
//! Every utterance is cancellable: the user can cut the console off
//! (barge-in), and a newer announcement replaces an older one instead of
//! queueing behind it. Piper is run as a subprocess from its release archive
//! — not through `piper-rs`, whose `ort` binaries need glibc 2.38 (the same
//! wall that moved embeddings from fastembed to candle). Its PCM is played
//! through cpal, the output side of the mic we already open, so playback
//! needs no platform audio tool either.

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};
use crate::services::voice_service::{model_language, resample};

/// Last Piper release with prebuilt binaries for all three desktops.
const PIPER_RELEASE: &str = "2023.11.14-2";
const VOICES_RELEASE: &str = "v1.0.0";
/// Emit a download-progress event at most every this many bytes.
const PROGRESS_EVERY_BYTES: u64 = 2 * 1024 * 1024;
/// Piper voices are 22.05 kHz unless their config says otherwise.
const DEFAULT_VOICE_RATE: u32 = 22_050;
const EARCON_RATE: u32 = 16_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Piper,
    SpdSay,
    Say,
    Sapi,
    None,
}

impl Engine {
    pub fn name(self) -> &'static str {
        match self {
            Engine::Piper => "piper",
            Engine::SpdSay => "spd-say",
            Engine::Say => "say",
            Engine::Sapi => "sapi",
            Engine::None => "none",
        }
    }
}

/// Piper when its binary and voice are on disk, else whatever the OS ships.
pub fn pick_engine(piper_ready: bool, os: &str) -> Engine {
    if piper_ready {
        return Engine::Piper;
    }
    match os {
        "linux" => Engine::SpdSay,
        "macos" => Engine::Say,
        "windows" => Engine::Sapi,
        _ => Engine::None,
    }
}

pub fn engine_name() -> String {
    pick_engine(piper_ready(), std::env::consts::OS)
        .name()
        .to_string()
}

/// The voice `voice_speak` would use, once Piper is installed.
pub fn voice_if_ready() -> Option<String> {
    piper_ready().then(voice_name)
}

// --- Install -----------------------------------------------------------------

/// Release archive for this platform: `(file name, is zip)`.
pub fn piper_archive_for(os: &str, arch: &str) -> Option<(&'static str, bool)> {
    match (os, arch) {
        ("linux", "x86_64") => Some(("piper_linux_x86_64.tar.gz", false)),
        ("linux", "aarch64") => Some(("piper_linux_aarch64.tar.gz", false)),
        ("macos", "x86_64") => Some(("piper_macos_x64.tar.gz", false)),
        ("macos", "aarch64") => Some(("piper_macos_aarch64.tar.gz", false)),
        ("windows", "x86_64") => Some(("piper_windows_amd64.zip", true)),
        _ => None,
    }
}

/// Default voice per language. Medium-quality voices synthesize ~15× faster
/// than real time on a 4-core CPU; `AGENT_CONSOLE_TTS_VOICE` overrides.
pub fn default_voice(lang: &str) -> &'static str {
    if lang.starts_with("es") {
        "es_ES-davefx-medium"
    } else {
        "en_US-lessac-medium"
    }
}

pub fn voice_name() -> String {
    std::env::var("AGENT_CONSOLE_TTS_VOICE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default_voice(&model_language()).to_string())
}

/// Path of a voice inside the piper-voices repo, from its name:
/// `es_ES-davefx-medium` → `es/es_ES/davefx/medium/es_ES-davefx-medium`.
pub fn voice_repo_path(name: &str) -> Option<String> {
    let (rest, quality) = name.rsplit_once('-')?;
    let (locale, speaker) = rest.split_once('-')?;
    let (lang, _region) = locale.split_once('_')?;
    if [lang, locale, speaker, quality]
        .iter()
        .any(|p| p.is_empty())
    {
        return None;
    }
    Some(format!("{lang}/{locale}/{speaker}/{quality}/{name}"))
}

fn tts_dir() -> AppResult<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?;
    Ok(base.join("agent-console").join("tts"))
}

fn piper_binary() -> AppResult<PathBuf> {
    let exe = if cfg!(windows) { "piper.exe" } else { "piper" };
    Ok(tts_dir()?.join("piper").join(exe))
}

fn voice_paths(name: &str) -> AppResult<(PathBuf, PathBuf)> {
    let dir = tts_dir()?.join("voices");
    Ok((
        dir.join(format!("{name}.onnx")),
        dir.join(format!("{name}.onnx.json")),
    ))
}

pub fn piper_ready() -> bool {
    let Ok(bin) = piper_binary() else {
        return false;
    };
    let Ok((onnx, json)) = voice_paths(&voice_name()) else {
        return false;
    };
    bin.exists() && onnx.exists() && json.exists()
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
    /// "piper" (the engine) or "voice" — the pill shows one bar for both.
    stage: &'static str,
}

/// Download `url` to `dest` atomically (`.part` + rename), reporting progress
/// on `voice://model-progress` — the same event the Whisper download uses, so
/// the voice pill shows one bar for everything voice needs.
async fn download(app: &AppHandle, url: &str, dest: &Path, stage: &'static str) -> AppResult<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut resp = reqwest::get(url)
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| AppError::Other(format!("tts download failed: {e}")))?;
    let total = resp.content_length();
    let part = dest.with_extension("part");
    let mut file = std::fs::File::create(&part)?;
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| AppError::Other(format!("tts download interrupted: {e}")))?
    {
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        if downloaded - last_emit >= PROGRESS_EVERY_BYTES {
            last_emit = downloaded;
            let _ = app.emit(
                "voice://model-progress",
                Progress {
                    downloaded,
                    total,
                    stage,
                },
            );
        }
    }
    file.sync_all()?;
    drop(file);
    std::fs::rename(&part, dest)?;
    Ok(())
}

/// Unpack a Piper release archive into `into`. Entries that would escape the
/// directory are refused (tar does this itself; zip via `enclosed_name`).
pub fn extract_archive(archive: &Path, is_zip: bool, into: &Path) -> AppResult<()> {
    std::fs::create_dir_all(into)?;
    let file = std::fs::File::open(archive)?;
    if is_zip {
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| AppError::Other(format!("piper archive: {e}")))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| AppError::Other(format!("piper archive entry: {e}")))?;
            let Some(rel) = entry.enclosed_name() else {
                continue;
            };
            let out = into.join(rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&out)?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
        }
    } else {
        let gz = flate2::read::GzDecoder::new(file);
        tar::Archive::new(gz).unpack(into)?;
    }
    Ok(())
}

/// Fetch the Piper binary and the configured voice if either is missing.
/// Best-effort for callers: the platform engine keeps working without it.
pub async fn ensure_piper(app: &AppHandle) -> AppResult<()> {
    if piper_ready() {
        return Ok(());
    }
    let dir = tts_dir()?;
    let bin = piper_binary()?;
    if !bin.exists() {
        let (file, is_zip) = piper_archive_for(std::env::consts::OS, std::env::consts::ARCH)
            .ok_or_else(|| {
                AppError::Other(format!(
                    "no Piper build for {}/{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ))
            })?;
        let url =
            format!("https://github.com/rhasspy/piper/releases/download/{PIPER_RELEASE}/{file}");
        let archive = dir.join(file);
        download(app, &url, &archive, "piper").await?;
        let into = dir.clone();
        tokio::task::spawn_blocking(move || extract_archive(&archive, is_zip, &into))
            .await
            .map_err(|e| AppError::Other(format!("piper extract task panicked: {e}")))??;
        let _ = std::fs::remove_file(dir.join(file));
        if !bin.exists() {
            return Err(AppError::Other(
                "piper archive did not contain the binary".into(),
            ));
        }
    }
    let name = voice_name();
    let (onnx, json) = voice_paths(&name)?;
    if !onnx.exists() || !json.exists() {
        let repo = voice_repo_path(&name)
            .ok_or_else(|| AppError::Other(format!("unrecognized voice name '{name}'")))?;
        let base =
            format!("https://huggingface.co/rhasspy/piper-voices/resolve/{VOICES_RELEASE}/{repo}");
        download(app, &format!("{base}.onnx.json"), &json, "voice").await?;
        download(app, &format!("{base}.onnx"), &onnx, "voice").await?;
    }
    Ok(())
}

fn voice_sample_rate(json_path: &Path) -> u32 {
    std::fs::read_to_string(json_path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["audio"]["sample_rate"].as_u64())
        .map(|r| r as u32)
        .unwrap_or(DEFAULT_VOICE_RATE)
}

// --- Text shaping -------------------------------------------------------------

/// Turn agent prose into something worth hearing: no fences, no markup, no
/// bullets, one line. Code blocks become a spoken placeholder.
pub fn speakable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            if !in_fence {
                out.push_str(" (código omitido) ");
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // Headings, bullets, numbered items, quotes.
        let t = t.trim_start_matches('#').trim_start();
        let t = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .or_else(|| t.strip_prefix("> "))
            .unwrap_or(t);
        let t = match t.find(". ") {
            Some(i) if i <= 2 && t[..i].chars().all(|c| c.is_ascii_digit()) => &t[i + 2..],
            _ => t,
        };
        out.push_str(t);
        out.push(' ');
    }
    // Links keep their text; inline code keeps its content; emphasis goes.
    let mut s = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' | '*' | '_' | '~' => {}
            '[' => {
                let mut label = String::new();
                let mut closed = false;
                for d in chars.by_ref() {
                    if d == ']' {
                        closed = true;
                        break;
                    }
                    label.push(d);
                }
                s.push_str(&label);
                if closed && chars.peek() == Some(&'(') {
                    for d in chars.by_ref() {
                        if d == ')' {
                            break;
                        }
                    }
                }
            }
            _ => s.push(c),
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// --- Speaking -----------------------------------------------------------------

struct Utterance {
    stop: Arc<AtomicBool>,
    child: Option<Arc<Mutex<Child>>>,
}

/// Cheaply cloneable (Arc'd) so commands can move it into spawn_blocking.
#[derive(Clone, Default)]
pub struct TtsService {
    current: Arc<Mutex<Option<Utterance>>>,
}

impl TtsService {
    /// Cut the current utterance short, if any. Safe to call when idle.
    pub fn stop(&self) {
        if let Some(u) = self.current.lock().take() {
            u.stop.store(true, Ordering::Relaxed);
            if let Some(child) = u.child {
                let _ = child.lock().kill();
            }
        }
    }

    /// Blocking: say `text` and return once it has been heard. `Ok(false)`
    /// means it was cut short (by `stop`, or by a newer `speak`), so a caller
    /// that meant to listen afterwards knows the question never landed.
    pub fn speak(&self, text: &str) -> AppResult<bool> {
        let text = speakable(text);
        if text.is_empty() {
            return Ok(true);
        }
        self.stop();
        let stop = Arc::new(AtomicBool::new(false));
        let engine = pick_engine(piper_ready(), std::env::consts::OS);
        tracing::info!(
            "tts: speaking via {} ({} chars)",
            engine.name(),
            text.chars().count()
        );
        let result = match engine {
            Engine::Piper => self.speak_piper(&text, stop.clone()),
            Engine::None => Err(AppError::Other(
                "no text-to-speech engine on this platform".into(),
            )),
            other => self.speak_platform(other, &text, stop.clone()),
        };
        // Only clear our own utterance: a newer speak() may already own the slot.
        {
            let mut cur = self.current.lock();
            if cur.as_ref().is_some_and(|u| Arc::ptr_eq(&u.stop, &stop)) {
                *cur = None;
            }
        }
        match result {
            Ok(()) => Ok(!stop.load(Ordering::Relaxed)),
            // A kill on purpose is not an error.
            Err(_) if stop.load(Ordering::Relaxed) => Ok(false),
            Err(e) => {
                tracing::warn!("tts: {} failed: {e}", engine.name());
                Err(e)
            }
        }
    }

    fn speak_piper(&self, text: &str, stop: Arc<AtomicBool>) -> AppResult<()> {
        let bin = piper_binary()?;
        let (onnx, json) = voice_paths(&voice_name())?;
        let rate = voice_sample_rate(&json);
        let mut cmd = Command::new(&bin);
        cmd.arg("--model")
            .arg(&onnx)
            .arg("--output-raw")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(dir) = bin.parent() {
            cmd.current_dir(dir);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::Other(format!("piper: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
            let _ = stdin.write_all(b"\n");
        }
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Other("piper: no stdout".into()))?;
        let child = Arc::new(Mutex::new(child));
        *self.current.lock() = Some(Utterance {
            stop: stop.clone(),
            child: Some(child.clone()),
        });

        let (tx, rx) = mpsc::channel::<Vec<i16>>();
        let player = start_player(rx, rate, stop.clone())?;
        let mut buf = [0u8; 8192];
        let mut carry: Option<u8> = None;
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let n = match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            let mut bytes: Vec<u8> = Vec::with_capacity(n + 1);
            if let Some(b) = carry.take() {
                bytes.push(b);
            }
            bytes.extend_from_slice(&buf[..n]);
            if bytes.len() % 2 == 1 {
                carry = bytes.pop();
            }
            let samples: Vec<i16> = bytes
                .chunks_exact(2)
                .map(|p| i16::from_le_bytes([p[0], p[1]]))
                .collect();
            if tx.send(samples).is_err() {
                break;
            }
        }
        drop(tx);
        let status = child.lock().wait();
        let played = player.join().unwrap_or(Ok(()));
        match status {
            Ok(s) if s.success() => played,
            Ok(s) => Err(AppError::Other(format!("piper exited with {s}"))),
            Err(e) => Err(AppError::Other(format!("piper: {e}"))),
        }
    }

    fn speak_platform(&self, engine: Engine, text: &str, stop: Arc<AtomicBool>) -> AppResult<()> {
        let lang = model_language();
        let (mut cmd, via_stdin) = match engine {
            Engine::SpdSay => {
                let mut c = Command::new("spd-say");
                c.args(["-w", "-l", &lang]).arg(text);
                (c, false)
            }
            Engine::Say => {
                let mut c = Command::new("say");
                c.args(["-f", "-"]);
                (c, true)
            }
            Engine::Sapi => {
                let mut c = Command::new("powershell");
                c.args([
                    "-NoProfile",
                    "-Command",
                    "Add-Type -AssemblyName System.Speech; \
                     (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak([Console]::In.ReadToEnd())",
                ]);
                (c, true)
            }
            Engine::Piper | Engine::None => unreachable!("handled by speak()"),
        };
        cmd.stdin(if via_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::Other(format!(
                    "{} not found — install it or enable Piper",
                    engine.name()
                ))
            } else {
                AppError::Other(format!("tts: {e}"))
            }
        })?;
        if via_stdin {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
        }
        let child = Arc::new(Mutex::new(child));
        *self.current.lock() = Some(Utterance {
            stop,
            child: Some(child.clone()),
        });
        // Poll instead of holding the lock in wait(): stop() needs it to kill.
        loop {
            let done = child
                .lock()
                .try_wait()
                .map_err(|e| AppError::Other(format!("tts: {e}")))?;
            match done {
                Some(s) if s.success() => return Ok(()),
                Some(s) => {
                    return Err(AppError::Other(format!(
                        "{} exited with {s}",
                        engine.name()
                    )))
                }
                None => std::thread::sleep(Duration::from_millis(40)),
            }
        }
    }

    /// A short tone: "listen" when the mic opens, "done" when it closes,
    /// "ack" after a voice command ran, "error" when one could not.
    pub fn earcon(&self, kind: &str) -> AppResult<()> {
        let (freq, ms) = match kind {
            "listen" => (880.0, 90),
            "done" => (660.0, 90),
            "ack" => (1046.0, 60),
            "error" => (220.0, 180),
            other => {
                return Err(AppError::InvalidArgument(format!(
                    "unknown earcon '{other}'"
                )))
            }
        };
        let samples = tone(freq, ms, EARCON_RATE);
        let (tx, rx) = mpsc::channel::<Vec<i16>>();
        let _ = tx.send(samples);
        drop(tx);
        let stop = Arc::new(AtomicBool::new(false));
        start_player(rx, EARCON_RATE, stop)?
            .join()
            .unwrap_or(Ok(()))
    }
}

/// Sine burst with a 5 ms fade at both ends so it doesn't click.
fn tone(freq: f32, ms: u32, rate: u32) -> Vec<i16> {
    let n = (rate * ms / 1000) as usize;
    let fade = (rate / 200) as usize; // 5 ms
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let env = if i < fade {
                i as f32 / fade as f32
            } else if i + fade > n {
                (n - i) as f32 / fade as f32
            } else {
                1.0
            };
            (0.35 * env * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

// --- Playback -----------------------------------------------------------------

/// Play mono i16 PCM arriving on `rx` (at `rate`) through the default output
/// device until the channel closes and the queue drains, or `stop` is set.
/// cpal streams are !Send, so the stream lives on the spawned thread; the
/// returned handle yields the open error, if any, when joined.
fn start_player(
    rx: mpsc::Receiver<Vec<i16>>,
    rate: u32,
    stop: Arc<AtomicBool>,
) -> AppResult<std::thread::JoinHandle<AppResult<()>>> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let handle = std::thread::spawn(move || player_thread(rx, rate, stop, ready_tx));
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(AppError::Other(e))
        }
        Err(_) => Err(AppError::Other("audio output init timed out".into())),
    }
}

fn player_thread(
    rx: mpsc::Receiver<Vec<i16>>,
    rate: u32,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), String>>,
) -> AppResult<()> {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        let _ = ready.send(Err("no audio output device".into()));
        return Err(AppError::Other("no audio output device".into()));
    };
    let supported = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(format!("audio output config: {e}")));
            return Err(AppError::Other(format!("audio output config: {e}")));
        }
    };
    let out_rate = supported.sample_rate();
    let channels = supported.channels().max(1) as usize;
    let config = supported.config();
    let queue: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));

    let built = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            let q = queue.clone();
            device.build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut q = q.lock();
                    for frame in data.chunks_mut(channels) {
                        let s = q.pop_front().unwrap_or(0.0);
                        frame.fill(s);
                    }
                },
                |e| tracing::warn!("tts: output stream error: {e}"),
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let q = queue.clone();
            device.build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut q = q.lock();
                    for frame in data.chunks_mut(channels) {
                        let s = (q.pop_front().unwrap_or(0.0) * 32767.0) as i16;
                        frame.fill(s);
                    }
                },
                |e| tracing::warn!("tts: output stream error: {e}"),
                None,
            )
        }
        other => {
            let msg = format!("unsupported output sample format: {other:?}");
            let _ = ready.send(Err(msg.clone()));
            return Err(AppError::Other(msg));
        }
    };
    let stream = match built {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(format!("audio output open failed: {e}")));
            return Err(AppError::Other(format!("audio output open failed: {e}")));
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("audio output start failed: {e}")));
        return Err(AppError::Other(format!("audio output start failed: {e}")));
    }
    let _ = ready.send(Ok(()));

    while let Ok(chunk) = rx.recv() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let f: Vec<f32> = chunk.iter().map(|&s| s as f32 / 32768.0).collect();
        let f = resample(&f, rate, out_rate);
        queue.lock().extend(f);
    }
    // Drain what is queued unless cut short; then give the device its
    // buffered tail before the stream drops.
    while !stop.load(Ordering::Relaxed) && !queue.lock().is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    if stop.load(Ordering::Relaxed) {
        queue.lock().clear();
    }
    std::thread::sleep(Duration::from_millis(60));
    drop(stream);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piper_wins_when_installed_else_the_platform_engine() {
        assert_eq!(pick_engine(true, "linux"), Engine::Piper);
        assert_eq!(pick_engine(true, "windows"), Engine::Piper);
        assert_eq!(pick_engine(false, "linux"), Engine::SpdSay);
        assert_eq!(pick_engine(false, "macos"), Engine::Say);
        assert_eq!(pick_engine(false, "windows"), Engine::Sapi);
        assert_eq!(pick_engine(false, "freebsd"), Engine::None);
    }

    #[test]
    fn release_archive_per_platform() {
        assert_eq!(
            piper_archive_for("linux", "x86_64"),
            Some(("piper_linux_x86_64.tar.gz", false))
        );
        assert_eq!(
            piper_archive_for("macos", "aarch64"),
            Some(("piper_macos_aarch64.tar.gz", false))
        );
        assert_eq!(
            piper_archive_for("windows", "x86_64"),
            Some(("piper_windows_amd64.zip", true))
        );
        assert_eq!(piper_archive_for("windows", "aarch64"), None);
    }

    #[test]
    fn voice_name_maps_to_its_repo_path() {
        assert_eq!(
            voice_repo_path("es_ES-davefx-medium").as_deref(),
            Some("es/es_ES/davefx/medium/es_ES-davefx-medium")
        );
        // Speakers may carry underscores; quality is always the last part.
        assert_eq!(
            voice_repo_path("en_US-hfc_female-medium").as_deref(),
            Some("en/en_US/hfc_female/medium/en_US-hfc_female-medium")
        );
        assert_eq!(voice_repo_path("davefx"), None);
        assert_eq!(voice_repo_path("es-davefx-medium"), None);
        assert_eq!(default_voice("es"), "es_ES-davefx-medium");
        assert_eq!(default_voice("en"), "en_US-lessac-medium");
    }

    #[test]
    fn speakable_strips_markup_and_hides_code() {
        let text = "## Done\n\n- Changed `src/a.rs` and **b.rs**.\n1. Ran tests\n```rust\nfn x() {}\n```\nSee [the PR](https://x/y) now.";
        assert_eq!(
            speakable(text),
            "Done Changed src/a.rs and b.rs. Ran tests (código omitido) See the PR now."
        );
        assert_eq!(speakable("   \n\n"), "");
    }

    /// Real audio: run with `cargo test plays_a_cue -- --ignored` on a machine
    /// with speakers to hear the "listen" cue and catch output-device errors.
    #[test]
    #[ignore]
    fn plays_a_cue_on_the_default_output() {
        TtsService::default().earcon("listen").unwrap();
        TtsService::default().earcon("done").unwrap();
    }

    /// Real audio through Piper (needs it installed under the data dir):
    /// `cargo test speaks_through -- --ignored`.
    #[test]
    #[ignore]
    fn speaks_through_the_installed_engine() {
        assert!(piper_ready(), "Piper + voice not installed");
        let t = TtsService::default();
        let heard = t.speak("Hola. Soy la consola y ya tengo voz.").unwrap();
        assert!(heard, "finished without interruption");
    }

    #[test]
    fn tone_is_bounded_and_faded() {
        let s = tone(440.0, 100, 16_000);
        assert_eq!(s.len(), 1600);
        assert_eq!(s[0], 0, "starts silent");
        assert!(
            s.iter().all(|&v| v.unsigned_abs() <= 11500),
            "0.35 amplitude"
        );
        assert!(
            s.iter().any(|&v| v.unsigned_abs() > 10_000),
            "reaches full amplitude"
        );
    }

    #[test]
    fn extracts_tar_gz_and_zip_into_the_target_dir() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("ac-tts-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();

        // tar.gz with piper/piper inside.
        let tgz = base.join("p.tar.gz");
        {
            let f = std::fs::File::create(&tgz).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            let data = b"#!/bin/sh\necho piper\n";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            tar.append_data(&mut h, "piper/piper", &data[..]).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let out1 = base.join("out1");
        extract_archive(&tgz, false, &out1).unwrap();
        assert_eq!(
            std::fs::read(out1.join("piper").join("piper")).unwrap(),
            b"#!/bin/sh\necho piper\n"
        );

        // zip with piper/piper.exe and an escaping entry that must be dropped.
        let zp = base.join("p.zip");
        {
            let f = std::fs::File::create(&zp).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let opts: zip::write::SimpleFileOptions = Default::default();
            z.start_file("piper/piper.exe", opts).unwrap();
            z.write_all(b"MZ").unwrap();
            z.start_file("../escape.txt", opts).unwrap();
            z.write_all(b"nope").unwrap();
            z.finish().unwrap();
        }
        let out2 = base.join("out2");
        extract_archive(&zp, true, &out2).unwrap();
        assert_eq!(
            std::fs::read(out2.join("piper").join("piper.exe")).unwrap(),
            b"MZ"
        );
        assert!(
            !base.join("escape.txt").exists(),
            "zip entry escaping the dir is refused"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
