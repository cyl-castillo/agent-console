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
//!
//! Beyond detection, each missing tool carries its official installer command
//! for this OS. The UI never runs it behind the user's back: it is typed into
//! a visible terminal without Enter (the review-first contract), so a novice
//! is one keypress away and an expert can edit the line first.

use serde::Serialize;

use crate::services::proc;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub name: String,
    pub found: bool,
    /// First line of `<bin> --version`, when available.
    pub version: Option<String>,
    /// Official installer for this OS — present only when the tool is missing.
    pub fix_command: Option<String>,
    /// Caveat the UI shows next to the fix ("needs Node", "Debian/Ubuntu", …).
    pub fix_note: Option<String>,
}

/// Login state of an agent engine. `logged_in: None` means the question could
/// not be answered — CLI missing, too old, or the probe timed out — and the UI
/// must render that as "unknown", never as "logged out" (no false alarms).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineAuth {
    pub engine: String,
    pub logged_in: Option<bool>,
    /// Human detail when the CLI reports one ("Logged in using ChatGPT", …).
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub tools: Vec<ToolStatus>,
    pub auth: Vec<EngineAuth>,
}

/// Official install command for `bin` on `os` ("linux" | "macos" | "windows"),
/// plus an optional caveat. Kept as data so tests pin the catalog and the UI
/// stays dumb. `node_found` lets codex warn that its installer needs npm.
fn fix_for(bin: &str, os: &str, node_found: bool) -> (Option<String>, Option<String>) {
    let cmd = |s: &str| Some(s.to_string());
    match (bin, os) {
        ("claude", "windows") => (cmd("irm https://claude.ai/install.ps1 | iex"), None),
        ("claude", _) => (cmd("curl -fsSL https://claude.ai/install.sh | bash"), None),
        ("codex", _) => (
            cmd("npm install -g @openai/codex"),
            if node_found {
                None
            } else {
                Some("Needs Node first — install it above, then re-check.".to_string())
            },
        ),
        ("node", "windows") => (cmd("winget install OpenJS.NodeJS.LTS"), None),
        ("node", _) => (
            cmd("curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash && \\. \"$HOME/.nvm/nvm.sh\" && nvm install 22"),
            Some("Installs nvm (the official Node version manager) in your home — no sudo.".to_string()),
        ),
        ("git", "windows") => (cmd("winget install Git.Git"), None),
        ("git", "macos") => (cmd("xcode-select --install"), None),
        ("git", _) => (
            cmd("sudo apt-get install -y git"),
            Some("Debian/Ubuntu — on other distros use your package manager.".to_string()),
        ),
        _ => (None, None),
    }
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
        fix_command: None,
        fix_note: None,
    }
}

#[cfg(not(windows))]
fn probe(bin: &str) -> ToolStatus {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    // `command -v` keeps the &&-chain honest: if the binary isn't on PATH the
    // chain short-circuits and the shell exits non-zero → found = false.
    let script =
        format!("command -v {bin} >/dev/null 2>&1 && {bin} --version 2>/dev/null | head -n1");
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

fn current_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// How long we wait for `codex login status` before answering "unknown".
const CODEX_AUTH_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// Bounded probe of codex's login state. Same contract as
/// `claude_cli::auth_status`: run off-thread, wait a bounded time, and let an
/// abandoned child exit on its own. Exit 0 = logged in; a clean non-zero exit
/// = logged out; anything else (missing bin, timeout) = unknown.
fn codex_auth() -> EngineAuth {
    #[cfg(not(windows))]
    let run = || {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        proc::command(&shell)
            .args(["-lc", "codex login status 2>&1"])
            .output()
    };
    #[cfg(windows)]
    let run = || proc::command("codex").args(["login", "status"]).output();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run());
    });
    let (logged_in, detail) = match rx.recv_timeout(CODEX_AUTH_PROBE_TIMEOUT) {
        Ok(Ok(o)) => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            // A shell that couldn't find the binary exits 127 with a "not
            // found" message — that is "unknown", not "logged out".
            if !o.status.success() && text.to_lowercase().contains("not found") {
                (None, None)
            } else {
                (
                    Some(o.status.success()),
                    if text.is_empty() {
                        None
                    } else {
                        Some(text.lines().next().unwrap_or("").trim().to_string())
                    },
                )
            }
        }
        _ => (None, None),
    };
    EngineAuth {
        engine: "codex".to_string(),
        logged_in,
        detail,
    }
}

/// Claude-side auth as an EngineAuth row, reusing the existing bounded probe.
fn claude_auth() -> EngineAuth {
    match crate::services::claude_cli::auth_status() {
        Some(s) => EngineAuth {
            engine: "claude".to_string(),
            logged_in: Some(s.logged_in),
            detail: s.account.or(s.method),
        },
        None => EngineAuth {
            engine: "claude".to_string(),
            logged_in: None,
            detail: None,
        },
    }
}

#[tauri::command]
pub fn preflight_check() -> Preflight {
    let mut tools = vec![probe("claude"), probe("node"), probe("git"), probe("codex")];
    let node_found = tools.iter().any(|t| t.name == "node" && t.found);
    let os = current_os();
    for t in tools.iter_mut() {
        if !t.found {
            let (cmd, note) = fix_for(&t.name, os, node_found);
            t.fix_command = cmd;
            t.fix_note = note;
        }
    }
    // Auth only makes sense for engines that are installed: probing a missing
    // CLI would burn two timeouts to learn nothing.
    let mut auth = Vec::new();
    if tools.iter().any(|t| t.name == "claude" && t.found) {
        auth.push(claude_auth());
    }
    if tools.iter().any(|t| t.name == "codex" && t.found) {
        auth.push(codex_auth());
    }
    Preflight { tools, auth }
}

/// Structured login state from `claude auth status --json`. `null` means the
/// question couldn't be answered — CLI missing, or older than the 2.1.41 that
/// made `claude auth` scriptable — and callers must treat that as "unknown",
/// not as "logged out".
#[tauri::command]
pub fn claude_auth_status() -> Option<crate::services::claude_cli::AuthStatus> {
    crate::services::claude_cli::auth_status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_an_official_fix_on_every_os() {
        for os in ["linux", "macos", "windows"] {
            for bin in ["claude", "codex", "node", "git"] {
                let (cmd, _) = fix_for(bin, os, true);
                assert!(cmd.is_some(), "{bin} on {os} must have a fix command");
            }
        }
    }

    #[test]
    fn codex_fix_warns_when_node_is_missing() {
        let (_, note) = fix_for("codex", "linux", false);
        assert!(note.unwrap().contains("Node"));
        let (_, note_ok) = fix_for("codex", "linux", true);
        assert!(note_ok.is_none());
    }

    #[test]
    fn claude_installer_is_the_native_one_not_npm() {
        // npm install of claude-code is deprecated upstream (2.1.15) — the
        // catalog must never hand a novice the deprecated path.
        for os in ["linux", "macos", "windows"] {
            let (cmd, _) = fix_for("claude", os, true);
            assert!(
                !cmd.unwrap().contains("npm"),
                "claude fix on {os} must not use npm"
            );
        }
    }

    #[test]
    fn found_tools_carry_no_fix() {
        let t = status("git", true, "git version 2.43.0");
        assert!(t.fix_command.is_none());
        assert_eq!(t.version.as_deref(), Some("git version 2.43.0"));
    }
}
