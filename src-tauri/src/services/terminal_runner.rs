use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::thread;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
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

    /// Central lock accessor for the terminal map. Uses parking_lot, so a panic
    /// while the lock is held does not poison it and cascade into every later
    /// terminal operation. The map stays consistent: each handle is only touched
    /// under the lock.
    fn lock_terms(&self) -> parking_lot::MutexGuard<'_, HashMap<String, TerminalHandle>> {
        self.terms.lock()
    }

    /// Spawn a new PTY in `cwd` running the user's default shell.
    /// Generic over the runtime so tests can drive it with tauri's MockRuntime.
    #[allow(dead_code)]
    pub fn spawn<R: Runtime>(&self, app: AppHandle<R>, cwd: &Path) -> AppResult<String> {
        self.spawn_with_env(app, cwd, &[])
    }

    pub fn spawn_with_env<R: Runtime>(
        &self,
        app: AppHandle<R>,
        cwd: &Path,
        extra_env: &[(String, String)],
    ) -> AppResult<String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
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
        cmd.env("AGENT_CONSOLE", "1");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

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
                                TermOutput {
                                    id: id.clone(),
                                    data: chunk,
                                },
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

        self.lock_terms().insert(
            id.clone(),
            TerminalHandle {
                master: pair.master,
                writer,
                killer,
            },
        );
        Ok(id)
    }

    pub fn write(&self, id: &str, data: &[u8]) -> AppResult<()> {
        let mut terms = self.lock_terms();
        let term = terms
            .get_mut(id)
            .ok_or_else(|| AppError::InvalidArgument(format!("unknown terminal: {id}")))?;
        term.writer
            .write_all(data)
            .map_err(|e| AppError::Other(format!("write: {e}")))?;
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> AppResult<()> {
        let terms = self.lock_terms();
        let term = terms
            .get(id)
            .ok_or_else(|| AppError::InvalidArgument(format!("unknown terminal: {id}")))?;
        term.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| AppError::Other(format!("resize: {e}")))?;
        Ok(())
    }

    pub fn kill(&self, id: &str) -> AppResult<()> {
        let mut terms = self.lock_terms();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tauri::Listener;

    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app")
    }

    #[test]
    fn unknown_ids_error_cleanly() {
        let reg = TerminalRegistry::new();
        assert!(matches!(
            reg.write("nope", b"x"),
            Err(AppError::InvalidArgument(_))
        ));
        assert!(matches!(
            reg.resize("nope", 80, 24),
            Err(AppError::InvalidArgument(_))
        ));
        assert!(matches!(
            reg.kill("nope"),
            Err(AppError::InvalidArgument(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn default_shell_respects_shell_env() {
        let _env = crate::test_support::lock_env();
        let prev = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "/bin/test-shell");
        assert_eq!(default_shell(), "/bin/test-shell");
        match prev {
            Some(v) => std::env::set_var("SHELL", v),
            None => std::env::remove_var("SHELL"),
        }
    }

    /// Full lifecycle against a real PTY: spawn a shell, run a command whose
    /// output proves both that the shell executed it and that `extra_env`
    /// reached the child, resize, exit, and verify kill() semantics.
    #[cfg(unix)]
    #[test]
    fn spawn_write_exit_lifecycle() {
        let _env = crate::test_support::lock_env();
        let prev = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "/bin/sh");

        let app = mock_app();
        let (out_tx, out_rx) = mpsc::channel::<String>();
        let (exit_tx, exit_rx) = mpsc::channel::<()>();
        app.listen("term://output", move |ev| {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(ev.payload()) {
                let _ = out_tx.send(v["data"].as_str().unwrap_or_default().to_string());
            }
        });
        app.listen("term://exit", move |_| {
            let _ = exit_tx.send(());
        });

        let reg = TerminalRegistry::new();
        let id = reg
            .spawn_with_env(
                app.handle().clone(),
                &std::env::temp_dir(),
                &[("AC_TEST_MARKER".into(), "tanda-t".into())],
            )
            .expect("spawn");

        // $-expansion means the shell ran it; the value proves extra_env
        // landed. The echoed input line only contains the literal `$VAR`,
        // so matching the expansion can't false-positive on echo.
        reg.write(&id, b"echo marker-$AC_TEST_MARKER\n").unwrap();
        let mut seen = String::new();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline && !seen.contains("marker-tanda-t") {
            if let Ok(chunk) = out_rx.recv_timeout(Duration::from_millis(250)) {
                seen.push_str(&chunk);
            }
        }
        assert!(seen.contains("marker-tanda-t"), "PTY output was: {seen:?}");

        reg.resize(&id, 120, 40).expect("resize live terminal");

        reg.write(&id, b"exit\n").unwrap();
        exit_rx
            .recv_timeout(Duration::from_secs(15))
            .expect("term://exit after typing exit");

        // The handle stays registered until the frontend acks with kill();
        // kill is the removal, and a second kill is a clean error.
        reg.kill(&id).expect("kill removes the handle");
        assert!(matches!(reg.kill(&id), Err(AppError::InvalidArgument(_))));

        match prev {
            Some(v) => std::env::set_var("SHELL", v),
            None => std::env::remove_var("SHELL"),
        }
    }

    /// kill() must preempt a shell that would otherwise sit there forever.
    #[cfg(unix)]
    #[test]
    fn kill_preempts_a_running_shell() {
        let _env = crate::test_support::lock_env();
        let prev = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "/bin/sh");

        let app = mock_app();
        let (exit_tx, exit_rx) = mpsc::channel::<()>();
        app.listen("term://exit", move |_| {
            let _ = exit_tx.send(());
        });

        let reg = TerminalRegistry::new();
        let id = reg
            .spawn_with_env(app.handle().clone(), &std::env::temp_dir(), &[])
            .expect("spawn");
        reg.kill(&id).expect("kill");
        exit_rx
            .recv_timeout(Duration::from_secs(15))
            .expect("term://exit after kill");

        match prev {
            Some(v) => std::env::set_var("SHELL", v),
            None => std::env::remove_var("SHELL"),
        }
    }
}
