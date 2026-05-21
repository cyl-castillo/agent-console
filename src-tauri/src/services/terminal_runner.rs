use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::thread;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// Payload emitted to the frontend for each chunk of PTY output.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermOutput {
    pub id: String,
    pub data: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermExit {
    pub id: String,
    pub code: Option<i32>,
}

/// A live terminal. The child runs on its own waiter thread; we keep a
/// `ChildKiller` here so kill() can preempt the wait.
struct TerminalHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
}

#[derive(Default)]
pub struct TerminalRegistry {
    terms: Mutex<HashMap<String, TerminalHandle>>,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a new PTY in `cwd` running the user's default shell.
    pub fn spawn(&self, app: AppHandle, cwd: &Path) -> AppResult<String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| AppError::Other(format!("openpty: {e}")))?;

        let mut cmd = CommandBuilder::new(default_shell());
        cmd.cwd(cwd);
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", home);
        }
        cmd.env("TERM", "xterm-256color");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| AppError::Other(format!("spawn: {e}")))?;

        let killer = child.clone_killer();

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| AppError::Other(format!("take_writer: {e}")))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| AppError::Other(format!("clone_reader: {e}")))?;

        let id = Uuid::new_v4().to_string();

        // Reader thread — pumps bytes to the frontend as `term://output`.
        {
            let id = id.clone();
            let app = app.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                            let _ = app.emit(
                                "term://output",
                                TermOutput { id: id.clone(), data: chunk },
                            );
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        // Waiter thread — owns the Child, emits `term://exit` on shell exit.
        {
            let id = id.clone();
            let app = app.clone();
            thread::spawn(move || {
                let exit = child.wait().ok();
                let code = exit.and_then(|s| i32::try_from(s.exit_code()).ok());
                let _ = app.emit("term://exit", TermExit { id, code });
            });
        }

        self.terms.lock().unwrap().insert(
            id.clone(),
            TerminalHandle { master: pair.master, writer, killer },
        );
        Ok(id)
    }

    pub fn write(&self, id: &str, data: &[u8]) -> AppResult<()> {
        let mut terms = self.terms.lock().unwrap();
        let term = terms
            .get_mut(id)
            .ok_or_else(|| AppError::InvalidArgument(format!("unknown terminal: {id}")))?;
        term.writer
            .write_all(data)
            .map_err(|e| AppError::Other(format!("write: {e}")))?;
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> AppResult<()> {
        let terms = self.terms.lock().unwrap();
        let term = terms
            .get(id)
            .ok_or_else(|| AppError::InvalidArgument(format!("unknown terminal: {id}")))?;
        term.master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| AppError::Other(format!("resize: {e}")))?;
        Ok(())
    }

    pub fn kill(&self, id: &str) -> AppResult<()> {
        let mut terms = self.terms.lock().unwrap();
        let mut term = terms
            .remove(id)
            .ok_or_else(|| AppError::InvalidArgument(format!("unknown terminal: {id}")))?;
        let _ = term.killer.kill();
        Ok(())
    }
}

/// Picks the user's preferred shell, with cross-platform defaults.
fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}
