//! Production voice handler (Hito 7): turns a spoken request into a short,
//! speakable reply by shelling out to the agent. STT/TTS happen on the phone;
//! only text crosses the wire, and the reply is shaped to be read aloud.
//!
//! MVP scope: a one-shot `claude -p` answer in the active project's directory —
//! no session continuity and no tool use yet. Letting voice DRIVE the agent
//! (edits/commands) is deferred, because per the threat model anything dangerous
//! must sit behind the on-device second factor, which isn't wired yet.

use std::path::PathBuf;

use crate::error::{AppError, AppResult};
use crate::services::transport::VoiceHandler;

pub struct ClaudeVoiceHandler {
    /// Run the agent here so it has project context; None = no specific cwd.
    project_root: Option<PathBuf>,
}

impl ClaudeVoiceHandler {
    pub fn new(project_root: Option<PathBuf>) -> Self {
        Self { project_root }
    }
}

impl VoiceHandler for ClaudeVoiceHandler {
    fn respond(&self, _device_id: &str, utterance: &str) -> AppResult<String> {
        let prompt = format!(
            "You are answering the user by VOICE in their software project. They said: \
             \"{}\".\nReply in 1-2 short sentences meant to be read aloud. No code blocks, \
             no markdown, no lists, no file paths unless essential — just what should be spoken.",
            utterance.replace('"', "'")
        );
        let mut cmd =
            crate::services::claude_cli::command(&["-p", &prompt, "--output-format", "text"]);
        if let Some(root) = &self.project_root {
            cmd.current_dir(root);
        }
        let out = cmd
            .output()
            .map_err(|e| AppError::Other(format!("failed to spawn `claude`: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(AppError::Other(format!(
                "claude exited with status {}: {}",
                out.status, stderr
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}
