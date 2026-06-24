//! First-run environment check. Agent Console drives external CLIs (claude /
//! codex) by typing them into a login-shell PTY, and the per-tool approval hook
//! is a Node script — so if those binaries are missing the user only finds out
//! via a cryptic `command not found` in a black terminal. This command probes
//! for them up front so the UI can warn *before* the wall.
//!
//! Resolution goes through a login shell (`$SHELL -lc`) on unix so the PATH
//! matches the interactive PTY: a GUI app launched from a desktop entry often
//! has a minimal PATH that omits nvm/npm-global binaries, which would otherwise
//! produce false "not found" results.

use serde::Serialize;

use crate::services::proc;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub name: String,
    pub found: bool,
    /// First line of `<bin> --version`, when available.
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub tools: Vec<ToolStatus>,
}

fn status(name: &str, found: bool, raw: &str) -> ToolStatus {
    let line = raw.lines().next().unwrap_or("").trim();
    ToolStatus {
        name: name.to_string(),
        found,
        version: if found && !line.is_empty() {
            Some(line.to_string())
        } else {
            None
        },
    }
}

#[cfg(not(windows))]
fn probe(bin: &str) -> ToolStatus {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    // `command -v` keeps the &&-chain honest: if the binary isn't on PATH the
    // chain short-circuits and the shell exits non-zero → found = false.
    let script = format!("command -v {bin} >/dev/null 2>&1 && {bin} --version 2>/dev/null | head -n1");
    match proc::command(&shell).args(["-lc", &script]).output() {
        Ok(o) if o.status.success() => status(bin, true, &String::from_utf8_lossy(&o.stdout)),
        _ => status(bin, false, ""),
    }
}

#[cfg(windows)]
fn probe(bin: &str) -> ToolStatus {
    match proc::command(bin).arg("--version").output() {
        Ok(o) if o.status.success() => status(bin, true, &String::from_utf8_lossy(&o.stdout)),
        _ => status(bin, false, ""),
    }
}

#[tauri::command]
pub fn preflight_check() -> Preflight {
    Preflight {
        tools: vec![
            probe("claude"),
            probe("node"),
            probe("git"),
            probe("codex"),
        ],
    }
}
