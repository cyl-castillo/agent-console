//! Resolving and spawning the `claude` CLI.
//!
//! A GUI app launched from a desktop/dock entry does NOT inherit the user's
//! login-shell PATH (the integrated terminal works only because it spawns a
//! login shell). So `Command::new("claude")` fails with "No such file or
//! directory (os error 2)" even though `claude` is on the user's PATH in a
//! normal terminal. We resolve the absolute path once and reuse it.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Caches a successfully-resolved absolute path. Failures are NOT cached, so a
/// claude installed after a failed lookup is picked up without an app restart.
static CACHED: Mutex<Option<String>> = Mutex::new(None);

/// Filenames to probe in each directory (Windows ships a `.cmd`/`.exe` shim).
#[cfg(windows)]
const NAMES: &[&str] = &["claude.cmd", "claude.exe", "claude.bat", "claude"];
#[cfg(not(windows))]
const NAMES: &[&str] = &["claude"];

/// Absolute path to the `claude` binary, or the bare name "claude" as a last
/// resort (so the caller still fails with a helpful "Is it on PATH?" message).
pub fn bin() -> String {
    if let Some(p) = CACHED.lock().unwrap().clone() {
        return p;
    }
    match resolve() {
        Some(p) => {
            *CACHED.lock().unwrap() = Some(p.clone());
            p
        }
        None => "claude".to_string(),
    }
}

/// A `claude <args>` command with stdio piped, stdin nulled (so it can never
/// block waiting for input), and — on Windows — no flashing console window.
pub fn command(args: &[&str]) -> Command {
    let mut cmd = Command::new(bin());
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn resolve() -> Option<String> {
    // 1. Explicit override — escape hatch for unusual installs.
    if let Ok(p) = std::env::var("AGENT_CONSOLE_CLAUDE_BIN") {
        let p = p.trim();
        if !p.is_empty() && Path::new(p).is_file() {
            return Some(p.to_string());
        }
    }
    // 2. Whatever PATH we did inherit (works when launched from a terminal).
    if let Some(p) = which_in_path() {
        return Some(p);
    }
    // 3. Ask the user's login shell — sources ~/.profile, ~/.bashrc, nvm, etc.
    //    This is the reliable path for a GUI launch on macOS/Linux.
    #[cfg(unix)]
    if let Some(p) = login_shell_which() {
        return Some(p);
    }
    // 4. Last resort: probe common install locations directly.
    common_locations()
}

/// Search the inherited PATH for any of NAMES.
fn which_in_path() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in NAMES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Resolve via `$SHELL -lc 'command -v claude'`, which loads the user's profile
/// and prints the absolute path the way their terminal would see it.
#[cfg(unix)]
fn login_shell_which() -> Option<String> {
    // Try the user's shell first, then the usual suspects. On a macOS GUI launch
    // SHELL may be unset (default would be /bin/sh, which never sources the zsh
    // profile), so we also try zsh (mac default) and bash explicitly.
    let mut shells: Vec<String> = Vec::new();
    if let Ok(s) = std::env::var("SHELL") {
        if !s.trim().is_empty() { shells.push(s); }
    }
    for s in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        if !shells.iter().any(|x| x == s) && Path::new(s).exists() {
            shells.push(s.to_string());
        }
    }

    for shell in shells {
        // `-lic`: login + interactive, so BOTH profile files (.zprofile/
        // .bash_profile) AND rc files (.zshrc/.bashrc) are sourced — the latter
        // is where nvm/fnm/asdf typically put node (and thus claude) on PATH.
        // stdin is nulled so an rc that reads input can't hang us.
        let Ok(output) = Command::new(&shell)
            .arg("-lic")
            .arg("command -v claude")
            .stdin(Stdio::null())
            .output()
        else { continue };
        if !output.status.success() { continue; }
        // Take the last line that is an actual file — rc files may print banners.
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(p) = stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty() && Path::new(l).is_file())
        {
            return Some(p.to_string());
        }
    }
    None
}

/// Probe well-known install locations, in rough order of likelihood.
fn common_locations() -> Option<String> {
    let home = dirs::home_dir();
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(h) = &home {
        candidates.push(h.join(".local/bin/claude")); // native installer
        candidates.push(h.join(".claude/local/claude")); // claude local install
        candidates.push(h.join(".bun/bin/claude"));
        candidates.push(h.join(".npm-global/bin/claude"));
        candidates.push(h.join(".yarn/bin/claude"));
        candidates.push(h.join(".volta/bin/claude")); // volta
        candidates.push(h.join(".asdf/shims/claude")); // asdf
        // nvm/fnm install node per-version; scan for the newest that has claude.
        if let Some(p) = scan_version_manager(&h.join(".nvm/versions/node")) {
            candidates.push(p);
        }
        if let Some(p) = scan_version_manager(&h.join(".local/share/fnm/node-versions")) {
            candidates.push(p);
        }
        #[cfg(windows)]
        {
            if let Ok(appdata) = std::env::var("APPDATA") {
                candidates.push(Path::new(&appdata).join("npm").join("claude.cmd"));
                candidates.push(Path::new(&appdata).join("npm").join("claude.exe"));
            }
        }
    }
    #[cfg(not(windows))]
    {
        candidates.push(Path::new("/usr/local/bin/claude").to_path_buf());
        candidates.push(Path::new("/usr/bin/claude").to_path_buf());
        candidates.push(Path::new("/opt/homebrew/bin/claude").to_path_buf());
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().to_string())
}

/// Given a version-manager root holding per-version node dirs (e.g.
/// ~/.nvm/versions/node/v22.3.0/bin), return the newest version's claude if it
/// exists. "Newest" = lexicographically-greatest dir name, which matches
/// zero-padded-free semver closely enough for a fallback probe.
fn scan_version_manager(root: &Path) -> Option<std::path::PathBuf> {
    let mut versions: Vec<std::path::PathBuf> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    for ver in versions.into_iter().rev() {
        // fnm nests an `installation` dir; nvm puts bin directly under the version.
        for bin in [ver.join("bin/claude"), ver.join("installation/bin/claude")] {
            if bin.is_file() {
                return Some(bin);
            }
        }
    }
    None
}
