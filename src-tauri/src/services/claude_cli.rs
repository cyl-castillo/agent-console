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

use parking_lot::Mutex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

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

/// A `claude <args>` command with stdin piped, for callers that feed the
/// prompt over stdin (`claude -p` with no positional prompt reads it there).
/// Prompts must NOT travel as argv: on Windows `CreateProcess` caps the whole
/// command line at 32,767 chars — and the npm `claude.cmd` shim, spawned via
/// `cmd.exe`, at ~8k — so a grown prompt fails to spawn with os error 206
/// ("file name or extension is too long"). stdin has no such cap.
pub fn command_with_stdin(args: &[&str]) -> Command {
    spawn_command(bin(), args, Stdio::piped())
}

/// Spawn `cmd`, write `prompt` to its stdin, close it, and collect the output.
/// `claude -p` reads stdin until EOF before answering, so dropping the handle
/// right after the write is load-bearing, not just tidy. A failed write is
/// deliberately ignored: it means the child died before reading (bad flag,
/// missing binary), and `wait_with_output` surfaces its stderr — a far better
/// error than "broken pipe".
pub fn output_with_stdin(mut cmd: Command, prompt: &str) -> std::io::Result<std::process::Output> {
    use std::io::Write;
    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes());
    }
    child.wait_with_output()
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

/// Structured authentication state, straight from `claude auth status --json`
/// (scriptable since Claude Code 2.1.41). Absence of this — see `auth_status`
/// returning `None` — means "we could not tell", never "logged out".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub logged_in: bool,
    /// How the CLI is authenticated ("claude.ai", "console", …), when reported.
    pub method: Option<String>,
    /// Account label to show in the UI (email, falling back to org name).
    pub account: Option<String>,
}

/// How long we wait for the auth probe before giving up and reporting
/// "unknown". The command is local and answers in milliseconds; the bound only
/// exists so a wedged CLI can never freeze a failure path.
const AUTH_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// Parse `claude auth status --json` output. Tolerant by design: any shape we
/// don't recognise (a usage error from a CLI predating the subcommand, a future
/// schema, an empty pipe) yields `None` = unknown.
fn parse_auth_status(stdout: &str) -> Option<AuthStatus> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    // `loggedIn` is the one field we insist on: without it the payload isn't an
    // auth status and guessing would be worse than admitting we don't know.
    let logged_in = v.get("loggedIn")?.as_bool()?;
    let str_field = |k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(AuthStatus {
        logged_in,
        method: str_field("authMethod"),
        account: str_field("email").or_else(|| str_field("orgName")),
    })
}

/// Ask the CLI whether it is logged in. `None` when the answer isn't
/// trustworthy: binary missing, subcommand unsupported (CLI < 2.1.41), probe
/// timed out, or output we can't parse.
pub fn auth_status() -> Option<AuthStatus> {
    let (tx, rx) = std::sync::mpsc::channel();
    // Run the probe off-thread and bound only our own wait: an abandoned child
    // exits on its own, and we never block a caller behind a stuck CLI.
    std::thread::spawn(move || {
        let _ = tx.send(command(&["auth", "status", "--json"]).output());
    });
    let output = rx.recv_timeout(AUTH_PROBE_TIMEOUT).ok()?.ok()?;
    // Parse regardless of exit status: a logged-out CLI may well exit non-zero
    // while still printing a perfectly good `{"loggedIn": false}`.
    parse_auth_status(&String::from_utf8_lossy(&output.stdout))
}

/// Definitive auth diagnosis for a failed `claude` run, or `None` when the
/// probe can't prove anything. This is what closes the gap the text heuristics
/// leave: expired credentials have surfaced as unrelated-looking errors ("issue
/// with the selected model") across several 2.1.x builds, so a run that failed
/// while the CLI reports itself logged out gets named for what it is.
pub fn logged_out_hint() -> Option<String> {
    let status = auth_status()?;
    if status.logged_in {
        return None;
    }
    Some(" — `claude auth status` reports you are not logged in; run `claude auth login` in a terminal (or use \"Fix Claude login\") and retry".to_string())
}

/// One live Claude session as `claude agents --json` reports it (scriptable
/// since Claude Code 2.1.145). Only the fields we can act on are kept: the OS
/// process (how we prove which terminal it belongs to) and the session id (the
/// resume handle). Everything else in the payload is ignored, so new fields
/// upstream can't break the parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveAgent {
    pub pid: u32,
    pub session_id: String,
    /// `"interactive"` for a TUI session; absent on CLIs that don't report it.
    pub kind: Option<String>,
}

/// Same bound, same reason as [`AUTH_PROBE_TIMEOUT`]: the command answers in
/// well under a second, and the timeout only exists so a wedged CLI can't stall
/// the caller.
const AGENTS_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Parse `claude agents --json`. Tolerant by design: anything that isn't an
/// array of objects carrying a numeric `pid` and a plausible `sessionId` yields
/// no agents — a CLI predating the subcommand prints a usage error, and an
/// unknown shape must read as "nothing to bind", never as a guess.
fn parse_live_agents(stdout: &str) -> Vec<LiveAgent> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(stdout.trim()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|it| {
            let pid = u32::try_from(it.get("pid")?.as_u64()?).ok()?;
            let session_id = it.get("sessionId")?.as_str()?.trim();
            // The id ends up inside a `claude --resume <id>` typed into a PTY,
            // and this payload comes from another process — same rule as every
            // other session id we accept: uuid alphabet only.
            if !is_safe_session_id(session_id) {
                return None;
            }
            Some(LiveAgent {
                pid,
                session_id: session_id.to_string(),
                kind: it
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

/// Session ids are uuids; anything else never reaches a shell command line.
fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Live Claude sessions on this machine, or an empty list when we can't tell
/// (binary missing, CLI older than 2.1.145, probe timed out, output unparsable).
/// "Can't tell" and "none running" collapse on purpose: both mean there is
/// nothing to bind, and the caller treats the result as additive evidence only.
pub fn live_agents() -> Vec<LiveAgent> {
    let (tx, rx) = std::sync::mpsc::channel();
    // Off-thread like `auth_status`: an abandoned child exits on its own, and a
    // stuck CLI can never hold up the reconcile pass.
    std::thread::spawn(move || {
        let _ = tx.send(command(&["agents", "--json"]).output());
    });
    let Ok(Ok(output)) = rx.recv_timeout(AGENTS_PROBE_TIMEOUT) else {
        return Vec::new();
    };
    parse_live_agents(&String::from_utf8_lossy(&output.stdout))
}

/// Build the error message for a non-zero `claude -p` exit. Claude Code often
/// prints the actual reason (auth expiry, usage limits) to STDOUT, not stderr
/// — the old stderr-only message reduced a real "OAuth session expired and
/// could not be refreshed" to a bare "exit status 1:". Prefer stderr, fall
/// back to stdout, cap the length, and add the fix-it hint for auth failures.
pub fn exit_error(output: &std::process::Output) -> String {
    exit_error_with(output, logged_out_hint())
}

/// Pure core of `exit_error`: `auth_hint` is the structured verdict from
/// `logged_out_hint`, injected so the formatting can be tested without a CLI.
fn exit_error_with(output: &std::process::Output, auth_hint: Option<String>) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    let reason: String = raw.chars().take(300).collect();
    let mut msg = if reason.is_empty() {
        format!("claude exited with status {} (no output)", output.status)
    } else {
        format!("claude exited with status {}: {reason}", output.status)
    };
    if let Some(hint) = auth_hint {
        // The structured answer beats the text heuristic — say it and stop.
        msg.push_str(&hint);
        return msg;
    }
    let lower = reason.to_lowercase();
    if lower.contains("authenticate") || lower.contains("oauth") || lower.contains("logged in") {
        msg.push_str(" — run `claude auth login` in a terminal and log in again");
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fake_output(code: &str, stdout: &str, stderr: &str) -> std::process::Output {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "printf %s '{stdout}'; printf %s '{stderr}' >&2; exit {code}"
            ))
            .output()
            .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn output_with_stdin_feeds_the_whole_prompt_and_collects_stdout() {
        // Larger than the 32,767-char Windows argv cap that motivated the
        // helper: the prompt must reach the child intact via stdin.
        let prompt = "x".repeat(40_000);
        let mut cmd = Command::new("cat");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let out = output_with_stdin(cmd, &prompt).expect("spawns");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), prompt);
    }

    #[cfg(unix)]
    #[test]
    fn output_with_stdin_surfaces_the_childs_error_not_a_broken_pipe() {
        // A child that exits without reading stdin: the write is best-effort
        // and the caller still gets the real exit status + stderr.
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("echo boom >&2; exit 3")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let out = output_with_stdin(cmd, &"y".repeat(1_000_000)).expect("spawns");
        assert_eq!(out.status.code(), Some(3));
        assert!(String::from_utf8_lossy(&out.stderr).contains("boom"));
    }

    #[cfg(unix)]
    #[test]
    fn exit_error_prefers_stderr_but_surfaces_stdout_reasons() {
        let both = fake_output("1", "stdout says A", "stderr says B");
        assert!(exit_error_with(&both, None).contains("stderr says B"));

        // The real-world case: auth errors land on stdout with empty stderr.
        let auth = fake_output(
            "1",
            "Failed to authenticate: OAuth session expired and could not be refreshed",
            "",
        );
        let msg = exit_error_with(&auth, None);
        assert!(msg.contains("OAuth session expired"), "{msg}");
        assert!(msg.contains("log in again"), "{msg}");

        let silent = fake_output("1", "", "");
        assert!(exit_error_with(&silent, None).contains("no output"));
    }

    #[cfg(unix)]
    #[test]
    fn exit_error_names_a_logged_out_cli_even_when_the_text_blames_something_else() {
        // The drift this closes: expired credentials reported as a model error.
        let misleading = fake_output("1", "There was an issue with the selected model", "");
        let msg = exit_error_with(&misleading, Some(" — not logged in".to_string()));
        assert!(msg.contains("issue with the selected model"), "{msg}");
        assert!(msg.contains("not logged in"), "{msg}");
        // The structured verdict replaces the guess — no double hint.
        let both_signals = fake_output("1", "Failed to authenticate", "");
        let msg = exit_error_with(&both_signals, Some(" — not logged in".to_string()));
        assert_eq!(msg.matches(" — ").count(), 1, "{msg}");
    }

    #[test]
    fn parses_a_logged_in_status() {
        let st = parse_auth_status(
            r#"{"loggedIn":true,"authMethod":"claude.ai","email":"a@b.c","subscriptionType":"max"}"#,
        )
        .expect("parses");
        assert!(st.logged_in);
        assert_eq!(st.method.as_deref(), Some("claude.ai"));
        assert_eq!(st.account.as_deref(), Some("a@b.c"));
    }

    #[test]
    fn falls_back_to_org_name_and_tolerates_missing_optionals() {
        let st = parse_auth_status(r#"{"loggedIn":false,"orgName":"Acme","email":"  "}"#)
            .expect("parses");
        assert!(!st.logged_in);
        assert_eq!(st.method, None);
        assert_eq!(st.account.as_deref(), Some("Acme"));
    }

    #[test]
    fn parses_live_agents_and_keeps_only_what_we_act_on() {
        let raw = r#"[
          {"pid":3449881,"cwd":"/w/agent-console","kind":"interactive",
           "startedAt":1787874025488,"sessionId":"450aeb31-8079-498b-afab-1f0fab67b3e7",
           "name":"agent-console-37"},
          {"pid":42,"sessionId":"abc-123"}
        ]"#;
        let agents = parse_live_agents(raw);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].pid, 3449881);
        assert_eq!(agents[0].session_id, "450aeb31-8079-498b-afab-1f0fab67b3e7");
        assert_eq!(agents[0].kind.as_deref(), Some("interactive"));
        // A CLI that stops reporting `kind` still yields a usable agent.
        assert_eq!(agents[1].kind, None);
    }

    #[test]
    fn live_agent_entries_we_cannot_trust_are_dropped() {
        // Missing pid, missing/blank id, and — the one that matters — an id
        // that would not be safe to type into `claude --resume <id>`.
        let raw = r#"[
          {"cwd":"/w","sessionId":"no-pid"},
          {"pid":1,"sessionId":"   "},
          {"pid":2,"sessionId":"ok-id"},
          {"pid":3,"sessionId":"; rm -rf ~"},
          {"pid":4,"sessionId":"$(whoami)"},
          {"pid":-7,"sessionId":"negative-pid"},
          "not-an-object"
        ]"#;
        let agents = parse_live_agents(raw);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].session_id, "ok-id");
    }

    #[test]
    fn unparsable_agent_output_means_no_agents_not_a_guess() {
        // Old CLI (no `agents` subcommand), a future shape, silence.
        for raw in [
            "",
            "   ",
            "error: unknown command 'agents'",
            r#"{"agents":[]}"#,
            "[ truncated",
        ] {
            assert!(parse_live_agents(raw).is_empty(), "{raw:?}");
        }
    }

    #[test]
    fn unknown_shapes_are_unknown_not_logged_out() {
        // A CLI without the subcommand (usage error), a future schema, silence:
        // all must read as "can't tell", because callers act on `logged_in`.
        for raw in [
            "",
            "   ",
            "error: unknown command 'auth'",
            r#"{"status":"ok"}"#,
            r#"{"loggedIn":"yes"}"#,
            "{ truncated",
        ] {
            assert_eq!(parse_auth_status(raw), None, "{raw:?}");
        }
    }
}
