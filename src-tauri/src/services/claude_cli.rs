//! Resolving and spawning coding-agent CLIs (`claude`, `codex`).
//!
//! A GUI app launched from a desktop/dock entry does NOT inherit the user's
//! login-shell PATH (the integrated terminal works only because it spawns a
//! login shell). So `Command::new("claude")` fails with "No such file or
//! directory (os error 2)" even though `claude` is on the user's PATH in a
//! normal terminal. We resolve the absolute path once (per binary) and reuse it.
//!
//! The resolution strategy is identical for every agent; only the binary name
//! and a few install-location leaves differ. `bin()`/`command()` keep the
//! original `claude`-only API; `codex_bin()`/`codex_command_with_stdin()` are
//! the `codex` equivalents. Both route through the same parameterized resolver.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use parking_lot::Mutex;

/// Caches successfully-resolved absolute paths, keyed by binary base name.
/// Failures are NOT cached, so a binary installed after a failed lookup is
/// picked up without an app restart.
static CACHED: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// An agent CLI we know how to resolve and spawn.
struct AgentBin {
    /// Base name as typed on a terminal ("claude", "codex").
    base: &'static str,
    /// Env var that force-overrides resolution (escape hatch for odd installs).
    env_override: &'static str,
}

const CLAUDE: AgentBin = AgentBin {
    base: "claude",
    env_override: "AGENT_CONSOLE_CLAUDE_BIN",
};
const CODEX: AgentBin = AgentBin {
    base: "codex",
    env_override: "AGENT_CONSOLE_CODEX_BIN",
};

/// Filenames to probe for `base` in each directory (Windows ships shims).
#[cfg(windows)]
fn names_for(base: &str) -> Vec<String> {
    vec![
        format!("{base}.cmd"),
        format!("{base}.exe"),
        format!("{base}.bat"),
        base.to_string(),
    ]
}
#[cfg(not(windows))]
fn names_for(base: &str) -> Vec<String> {
    vec![base.to_string()]
}

/// Absolute path to the `claude` binary (see `resolve`), or the bare name as a
/// last resort so the caller still fails with a helpful "Is it on PATH?".
pub fn bin() -> String {
    resolve_cached(&CLAUDE)
}

/// Absolute path to the `codex` binary, resolved the same way as `claude`.
pub fn codex_bin() -> String {
    resolve_cached(&CODEX)
}

/// A `claude <args>` command with stdio piped, stdin nulled (so it can never
/// block waiting for input), and — on Windows — no flashing console window.
pub fn command(args: &[&str]) -> Command {
    spawn_command(bin(), args, Stdio::null())
}

/// A `codex <args>` command with stdin piped for callers that intentionally
/// feed the prompt over stdin instead of passing it as an argv value. Codex's
/// `exec` mode blocks until stdin is closed, so the caller must write and then
/// drop the stdin handle — leaving it open hangs the child.
pub fn codex_command_with_stdin(args: &[&str]) -> Command {
    spawn_command(codex_bin(), args, Stdio::piped())
}

fn spawn_command(program: String, args: &[&str], stdin: Stdio) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(stdin)
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

fn resolve_cached(agent: &AgentBin) -> String {
    if let Some(map) = CACHED.lock().as_ref() {
        if let Some(p) = map.get(agent.base) {
            return p.clone();
        }
    }
    match resolve(agent) {
        Some(p) => {
            CACHED
                .lock()
                .get_or_insert_with(HashMap::new)
                .insert(agent.base.to_string(), p.clone());
            p
        }
        None => agent.base.to_string(),
    }
}

fn resolve(agent: &AgentBin) -> Option<String> {
    // 1. Explicit override — escape hatch for unusual installs.
    if let Ok(p) = std::env::var(agent.env_override) {
        let p = p.trim();
        if !p.is_empty() && Path::new(p).is_file() {
            return Some(p.to_string());
        }
    }
    // 2. Whatever PATH we did inherit (works when launched from a terminal).
    if let Some(p) = which_in_path(agent.base) {
        return Some(p);
    }
    // 3. Ask the user's login shell — sources ~/.profile, ~/.bashrc, nvm, etc.
    //    This is the reliable path for a GUI launch on macOS/Linux.
    #[cfg(unix)]
    if let Some(p) = login_shell_which(agent.base) {
        return Some(p);
    }
    // 4. Last resort: probe common install locations directly.
    common_locations(agent.base)
}

/// Search the inherited PATH for any name variant of `base`.
fn which_in_path(base: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in names_for(base) {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Resolve via `$SHELL -lic 'command -v <base>'`, which loads the user's profile
/// and prints the absolute path the way their terminal would see it.
#[cfg(unix)]
fn login_shell_which(base: &str) -> Option<String> {
    // Try the user's shell first, then the usual suspects. On a macOS GUI launch
    // SHELL may be unset (default would be /bin/sh, which never sources the zsh
    // profile), so we also try zsh (mac default) and bash explicitly.
    let mut shells: Vec<String> = Vec::new();
    if let Ok(s) = std::env::var("SHELL") {
        if !s.trim().is_empty() {
            shells.push(s);
        }
    }
    for s in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        if !shells.iter().any(|x| x == s) && Path::new(s).exists() {
            shells.push(s.to_string());
        }
    }

    for shell in shells {
        // `-lic`: login + interactive, so BOTH profile files (.zprofile/
        // .bash_profile) AND rc files (.zshrc/.bashrc) are sourced — the latter
        // is where nvm/fnm/asdf typically put node (and thus the CLI) on PATH.
        // stdin is nulled so an rc that reads input can't hang us.
        let Ok(output) = Command::new(&shell)
            .arg("-lic")
            .arg(format!("command -v {base}"))
            .stdin(Stdio::null())
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
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
fn common_locations(base: &str) -> Option<String> {
    let home = dirs::home_dir();
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(h) = &home {
        candidates.push(h.join(format!(".local/bin/{base}"))); // native installer
        if base == "claude" {
            candidates.push(h.join(".claude/local/claude")); // claude local install
        }
        candidates.push(h.join(format!(".bun/bin/{base}")));
        candidates.push(h.join(format!(".npm-global/bin/{base}")));
        candidates.push(h.join(format!(".yarn/bin/{base}")));
        candidates.push(h.join(format!(".volta/bin/{base}"))); // volta
        candidates.push(h.join(format!(".asdf/shims/{base}"))); // asdf
                                                                // nvm/fnm install node per-version; scan for the newest that has it.
        if let Some(p) = scan_version_manager(&h.join(".nvm/versions/node"), base) {
            candidates.push(p);
        }
        if let Some(p) = scan_version_manager(&h.join(".local/share/fnm/node-versions"), base) {
            candidates.push(p);
        }
        #[cfg(windows)]
        {
            if let Ok(appdata) = std::env::var("APPDATA") {
                candidates.push(Path::new(&appdata).join("npm").join(format!("{base}.cmd")));
                candidates.push(Path::new(&appdata).join("npm").join(format!("{base}.exe")));
            }
        }
    }
    #[cfg(not(windows))]
    {
        candidates.push(Path::new(&format!("/usr/local/bin/{base}")).to_path_buf());
        candidates.push(Path::new(&format!("/usr/bin/{base}")).to_path_buf());
        candidates.push(Path::new(&format!("/opt/homebrew/bin/{base}")).to_path_buf());
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().to_string())
}

/// Given a version-manager root holding per-version node dirs (e.g.
/// ~/.nvm/versions/node/v22.3.0/bin), return the newest version's `base` binary
/// if it exists. "Newest" = lexicographically-greatest dir name, which matches
/// zero-padded-free semver closely enough for a fallback probe.
fn scan_version_manager(root: &Path, base: &str) -> Option<std::path::PathBuf> {
    let mut versions: Vec<std::path::PathBuf> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    for ver in versions.into_iter().rev() {
        // fnm nests an `installation` dir; nvm puts bin directly under the version.
        for bin in [
            ver.join(format!("bin/{base}")),
            ver.join(format!("installation/bin/{base}")),
        ] {
            if bin.is_file() {
                return Some(bin);
            }
        }
    }
    None
}
