//! Firewall mode (TESTIGO firewall, Phase 2): the console's hooks and a
//! baseline policy placed where the agent cannot edit them.
//!
//! Everything the console enforces today lives in files the agent's own
//! shell can write (same uid): `~/.claude/settings.json`, the bridge binary
//! under the cache dir, the answer files. Claude Code reads one tier above
//! all of that — managed settings, owned by the administrator — and nothing
//! in user, project, local or `--settings` scope overrides it. Firewall
//! mode is a drop-in in that tier plus a root-owned copy of the bridge:
//!
//! - the console's hooks, pointing at the root-owned bridge, with
//!   `allowManagedHooksOnly` so no other hook runs and `onFailure: "block"`
//!   on the two hooks that decide, so a crash, a timeout or garbage denies:
//!   the guard on every tool (`PreToolUse`, which sends a high-tier request
//!   to a person whatever the session's mode) and the approval hook;
//! - `allowManagedPermissionRulesOnly`, so an allow rule the agent writes
//!   into a settings file it can reach authorizes nothing;
//! - deny rules for the console's own state and the managed tier itself;
//! - the sandbox on, not skippable, and fatal when it cannot start: the
//!   agent's shell cannot write outside the project or reach loopback, which
//!   is what keeps it away from the answer files and the pickup endpoint.
//!
//! This module only *describes* that drop-in and *inspects* what is on
//! disk. It never elevates and never writes outside the user's home:
//! installing takes the administrator's password and is the user's own
//! action.
//!
//! Engine parity: Claude Code only. Codex and OpenCode have no tier this
//! module writes; the status says so for each.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::services::hooks_service;

/// Oldest Claude Code with `onFailure` on command hooks. Below it a hook
/// that crashes or times out lets the action through, which is the opposite
/// of what this mode promises.
pub const MIN_CLAUDE_FIREWALL_VERSION: (u64, u64, u64) = (2, 1, 295);

/// Sorts after an organization's own low-numbered drop-ins and before
/// anything deliberately placed last.
const DROPIN_NAME: &str = "50-agent-console.json";

/// Seconds the approval hook may wait for a human before Claude Code
/// cancels it (and, with `onFailure: "block"`, denies).
const APPROVAL_HOOK_TIMEOUT_S: u64 = 600;

/// Seconds the guard may take. It classifies and exits; anything slower is
/// a broken bridge, and under `onFailure: "block"` that denies.
const GUARD_HOOK_TIMEOUT_S: u64 = 10;

/// Hook events of the drop-in, with whether a failure must block. The guard
/// (PreToolUse) and the approval hook decide; the rest record.
const EVENTS: &[(&str, bool)] = &[
    ("UserPromptSubmit", false),
    ("PreToolUse", true),
    ("PermissionRequest", true),
    ("PostToolUse", false),
    ("PostToolUseFailure", false),
    ("Notification", false),
    ("Stop", false),
    ("StopFailure", false),
    ("PostModelSwitch", false),
];

/// Where the managed tier and the root-owned bridge live on this OS. `None`
/// where firewall mode is not offered (Windows: Claude Code's sandbox does
/// not run there, and without it the agent's shell reaches the answer
/// files whatever the hooks say).
pub fn locations() -> Option<(PathBuf, PathBuf)> {
    if cfg!(target_os = "linux") {
        Some((
            PathBuf::from("/etc/claude-code/managed-settings.d").join(DROPIN_NAME),
            PathBuf::from("/opt/agent-console").join(hooks_service::BRIDGE_BIN_NAME),
        ))
    } else if cfg!(target_os = "macos") {
        Some((
            PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.d")
                .join(DROPIN_NAME),
            PathBuf::from("/Library/Application Support/AgentConsole")
                .join(hooks_service::BRIDGE_BIN_NAME),
        ))
    } else {
        None
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// `~/…` when `path` is under `home` (the form Claude Code's permission
/// rules resolve per user), `//…` (absolute) otherwise.
fn rule_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(rel) = home.and_then(|h| path.strip_prefix(h).ok()) {
        return format!("~/{}", rel.to_string_lossy());
    }
    format!("/{}", path.to_string_lossy())
}

/// Files no tool may edit while the mode is on: the console's data (ledger,
/// trust, sponsors, answer files), its cache (the unprivileged bridge copy)
/// and the managed tier itself. `Edit` rules cover every file-editing tool.
fn baseline_deny(dropin: &Path, bridge: &Path, home: Option<&Path>) -> Vec<String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = dirs::data_dir() {
        dirs.push(d.join("agent-console"));
    }
    if let Some(d) = dirs::cache_dir() {
        dirs.push(d.join("agent-console"));
    }
    // managed-settings.d → the directory that holds managed-settings.json too.
    if let Some(d) = dropin.parent().and_then(Path::parent) {
        dirs.push(d.to_path_buf());
    }
    if let Some(d) = bridge.parent() {
        dirs.push(d.to_path_buf());
    }
    dirs.iter()
        .map(|d| format!("Edit({}/**)", rule_path(d, home)))
        .collect()
}

/// The drop-in, for a Claude Code of `version`. Pure: same inputs, same
/// bytes, so the hash a user reviewed is the hash the status looks for.
pub fn dropin_settings(
    version: (u64, u64, u64),
    dropin: &Path,
    bridge: &Path,
    home: Option<&Path>,
) -> Value {
    let mut hooks = serde_json::Map::new();
    for (event, blocking) in EVENTS {
        if !hooks_service::claude_event_supported(event, Some(version)) {
            continue;
        }
        // PreToolUse is the guard, not the legacy approvals bridge
        // (`pretooluse`): that one answers every request itself, and then
        // PermissionRequest never sees one. The guard only escalates.
        let (mode, timeout) = if *event == "PreToolUse" {
            ("guard", GUARD_HOOK_TIMEOUT_S)
        } else {
            (hooks_service::bridge_mode(event), APPROVAL_HOOK_TIMEOUT_S)
        };
        let mut hook = json!({
            "type": "command",
            "command": format!("\"{}\" {mode}", bridge.to_string_lossy()),
        });
        if *blocking {
            hook["onFailure"] = json!("block");
            hook["timeout"] = json!(timeout);
        }
        hooks.insert(
            (*event).to_string(),
            json!([{ "matcher": "*", "hooks": [hook] }]),
        );
    }
    json!({
        "allowManagedHooksOnly": true,
        "allowManagedPermissionRulesOnly": true,
        "hooks": Value::Object(hooks),
        "permissions": {
            "deny": baseline_deny(dropin, bridge, home),
            "disableBypassPermissionsMode": "disable",
        },
        "sandbox": {
            "enabled": true,
            "allowUnsandboxedCommands": false,
            "failIfUnavailable": true,
        },
    })
}

/// The exact bytes to install: pretty JSON and a trailing newline.
pub fn render(settings: &Value) -> String {
    let mut s = serde_json::to_string_pretty(settings).unwrap_or_default();
    s.push('\n');
    s
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineCoverage {
    pub engine: String,
    pub covered: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FirewallStatus {
    /// `unsupported` · `off` · `enforced` · `drifted` · `unsafe`
    ///
    /// `unsafe` = files are there but someone other than the administrator
    /// can change them (or Claude Code cannot read them, which makes it
    /// start without the policy). `drifted` = safe to trust, but not what
    /// this version of the console would install.
    pub state: String,
    /// Why, in sentences a person can act on. Empty when `enforced` or `off`.
    pub problems: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropin_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_path: Option<String>,
    /// What the console would install, and its hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_sha256: Option<String>,
    /// Shell text the user reviews and runs in their own terminal to turn
    /// the mode on (or bring a drifted install up to date). Absent when
    /// this build ships no bridge binary to install.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_script: Option<String>,
    /// Shell text that removes the drop-in and the root-owned bridge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uninstall_script: Option<String>,
    pub engines: Vec<EngineCoverage>,
}

fn engines(claude_covered: bool) -> Vec<EngineCoverage> {
    vec![
        EngineCoverage {
            engine: "claude".into(),
            covered: claude_covered,
            note: if claude_covered {
                "Hooks, permission rules and sandbox come from managed settings.".into()
            } else {
                "Not enforced.".into()
            },
        },
        EngineCoverage {
            engine: "codex".into(),
            covered: false,
            note: "Not supported: firewall mode writes no Codex policy. Codex sessions run with the hooks and rules in your home.".into(),
        },
        EngineCoverage {
            engine: "opencode".into(),
            covered: false,
            note: "Not supported: OpenCode has no sandbox and no managed tier.".into(),
        },
    ]
}

fn unsupported(reason: String) -> FirewallStatus {
    FirewallStatus {
        state: "unsupported".into(),
        problems: vec![reason],
        dropin_path: None,
        bridge_path: None,
        expected: None,
        expected_sha256: None,
        install_script: None,
        uninstall_script: None,
        engines: engines(false),
    }
}

/// One argument for `sh`, single-quoted. `None` for a path that cannot be
/// carried on one line: the scripts are text a person reads before running,
/// and a line break inside an argument is how such text lies.
fn sh_quote(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    if s.is_empty() || s.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(format!("'{}'", s.replace('\'', "'\\''")))
}

const INSTALL_DELIM: &str = "AGENT_CONSOLE_INSTALL";
const DROPIN_DELIM: &str = "AGENT_CONSOLE_DROPIN";

/// The install, as shell text for the user's own terminal. The console
/// never runs it and never elevates.
///
/// Nothing root does here trusts a file the agent can write:
/// - the drop-in travels inside the text (a quoted here-document), not in
///   a staged file;
/// - the bridge is copied to a root-only name first, checked there against
///   the hash of the binary this build ships, and only then made
///   executable and moved into place — so a swapped source is refused, and
///   a source swapped for a link to someone's private file is never left
///   readable.
///
/// Both files land through a rename, so Claude Code never reads half of one.
pub fn install_script(
    dropin: &Path,
    bridge: &Path,
    expected: &str,
    bridge_source: &Path,
    bridge_sha256: &str,
    macos: bool,
) -> Option<String> {
    if !bridge_sha256.chars().all(|c| c.is_ascii_hexdigit()) || bridge_sha256.len() != 64 {
        return None;
    }
    if expected
        .lines()
        .any(|l| l == DROPIN_DELIM || l == INSTALL_DELIM)
    {
        return None;
    }
    let with_suffix = |p: &Path| {
        let mut os = p.as_os_str().to_os_string();
        os.push(".new");
        PathBuf::from(os)
    };
    let dropin_dir = sh_quote(dropin.parent()?)?;
    let bridge_dir = sh_quote(bridge.parent()?)?;
    let dropin_new = sh_quote(&with_suffix(dropin))?;
    let bridge_new = sh_quote(&with_suffix(bridge))?;
    let dropin_q = sh_quote(dropin)?;
    let bridge_q = sh_quote(bridge)?;
    let source = sh_quote(bridge_source)?;
    let sha_check = if macos {
        "shasum -a 256 -c -"
    } else {
        "sha256sum -c -"
    };
    // `sha256sum -c` reads "<hash>  <path>"; the path is the root-only copy.
    let check_line = sh_quote(Path::new(&format!(
        "{bridge_sha256}  {}",
        with_suffix(bridge).to_str()?
    )))?;
    Some(format!(
        "sh -eu <<'{INSTALL_DELIM}'\n\
         sudo install -d -o 0 -g 0 -m 0755 {dropin_dir} {bridge_dir}\n\
         sudo install -o 0 -g 0 -m 0700 {source} {bridge_new}\n\
         echo {check_line} | sudo {sha_check}\n\
         sudo chmod 0755 {bridge_new}\n\
         sudo mv -f {bridge_new} {bridge_q}\n\
         sudo tee {dropin_new} >/dev/null <<'{DROPIN_DELIM}'\n\
         {expected}\
         {DROPIN_DELIM}\n\
         sudo chown 0:0 {dropin_new}\n\
         sudo chmod 0644 {dropin_new}\n\
         sudo mv -f {dropin_new} {dropin_q}\n\
         {INSTALL_DELIM}\n"
    ))
}

/// Turning the mode off: the drop-in first, so Claude Code never sees
/// hooks that point at a bridge that is gone (which would deny everything).
pub fn uninstall_script(dropin: &Path, bridge: &Path) -> Option<String> {
    Some(format!(
        "sudo rm -f {} && sudo rm -f {}\n",
        sh_quote(dropin)?,
        sh_quote(bridge)?
    ))
}

/// Can anyone but `admin_uid` change `path` — by writing it, or by
/// replacing it through a directory between it and `anchor`? Also rejects
/// a symlink at `path` itself (its target is a second thing to trust).
#[cfg(unix)]
fn ownership_problems(path: &Path, anchor: &Path, admin_uid: u32) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let mut out = Vec::new();
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            out.push(format!("{} is a symlink", path.display()));
        }
        Ok(_) => {}
        Err(e) => out.push(format!("{}: {e}", path.display())),
    }
    let mut cur = Some(path);
    while let Some(p) = cur {
        if let Ok(m) = std::fs::metadata(p) {
            if m.uid() != admin_uid {
                out.push(format!(
                    "{} is owned by uid {}, not the administrator",
                    p.display(),
                    m.uid()
                ));
            }
            if m.mode() & 0o022 != 0 {
                out.push(format!(
                    "{} is writable by its group or by everyone",
                    p.display()
                ));
            }
        }
        if p == anchor {
            break;
        }
        cur = p.parent();
    }
    out
}

#[cfg(not(unix))]
fn ownership_problems(_path: &Path, _anchor: &Path, _admin_uid: u32) -> Vec<String> {
    Vec::new()
}

#[cfg(unix)]
fn world_readable(path: &Path, want_exec: bool) -> bool {
    use std::os::unix::fs::MetadataExt;
    let need = if want_exec { 0o005 } else { 0o004 };
    std::fs::metadata(path).is_ok_and(|m| m.mode() & need == need)
}

#[cfg(not(unix))]
fn world_readable(_path: &Path, _want_exec: bool) -> bool {
    true
}

/// What is installed, against what should be. `anchor` is where the
/// ownership walk stops (`/` in production) and `admin_uid` who may own the
/// files (0); both are parameters so a test can use a temp dir it owns.
pub fn inspect(
    dropin: &Path,
    bridge: &Path,
    expected: &str,
    bundled_bridge: Option<&Path>,
    anchor: &Path,
    admin_uid: u32,
) -> FirewallStatus {
    let mut status = FirewallStatus {
        state: "off".into(),
        problems: Vec::new(),
        dropin_path: Some(dropin.to_string_lossy().to_string()),
        bridge_path: Some(bridge.to_string_lossy().to_string()),
        expected: Some(expected.to_string()),
        expected_sha256: Some(sha256_hex(expected.as_bytes())),
        install_script: None,
        uninstall_script: None,
        engines: engines(false),
    };
    if std::fs::symlink_metadata(dropin).is_err() {
        return status;
    }

    let mut unsafe_problems = ownership_problems(dropin, anchor, admin_uid);
    if !world_readable(dropin, false) {
        unsafe_problems.push(format!(
            "{} is not readable by every user: Claude Code starts without a policy it cannot read",
            dropin.display()
        ));
    }
    if std::fs::symlink_metadata(bridge).is_ok() {
        unsafe_problems.extend(ownership_problems(bridge, anchor, admin_uid));
        if !world_readable(bridge, true) {
            unsafe_problems.push(format!(
                "{} is not executable by every user",
                bridge.display()
            ));
        }
    }
    if !unsafe_problems.is_empty() {
        status.state = "unsafe".into();
        status.problems = unsafe_problems;
        return status;
    }

    let mut drift = Vec::new();
    match std::fs::read(dropin) {
        Ok(bytes) if bytes == expected.as_bytes() => {}
        Ok(_) => drift.push(format!(
            "{} is not what this version of Agent Console installs",
            dropin.display()
        )),
        Err(e) => drift.push(format!("{}: {e}", dropin.display())),
    }
    match std::fs::read(bridge) {
        Err(_) => drift.push(format!(
            "{} is missing: every approval would be denied",
            bridge.display()
        )),
        Ok(installed) => {
            // No bundled sidecar (a dev run) ⇒ nothing to compare against.
            if let Some(bundled) = bundled_bridge.and_then(|p| std::fs::read(p).ok()) {
                if sha256_hex(&installed) != sha256_hex(&bundled) {
                    drift.push(format!(
                        "{} is from another version of Agent Console",
                        bridge.display()
                    ));
                }
            }
        }
    }
    if drift.is_empty() {
        status.state = "enforced".into();
        status.engines = engines(true);
    } else {
        status.state = "drifted".into();
        status.problems = drift;
    }
    status
}

/// Firewall mode on this machine, for the installed Claude Code.
pub fn status() -> FirewallStatus {
    let Some((dropin, bridge)) = locations() else {
        return unsupported(
            "Firewall mode needs Claude Code's sandbox, which runs on Linux and macOS only.".into(),
        );
    };
    let Some(version) = hooks_service::detect_claude_version() else {
        return unsupported("Could not run `claude --version`.".into());
    };
    if version < MIN_CLAUDE_FIREWALL_VERSION {
        let (a, b, c) = MIN_CLAUDE_FIREWALL_VERSION;
        return unsupported(format!(
            "Claude Code {}.{}.{} lets an action through when a hook fails; firewall mode needs {a}.{b}.{c} or newer.",
            version.0, version.1, version.2
        ));
    }
    let home = dirs::home_dir();
    let expected = render(&dropin_settings(version, &dropin, &bridge, home.as_deref()));
    let bundled = hooks_service::bundled_bridge();
    let mut status = inspect(
        &dropin,
        &bridge,
        &expected,
        bundled.as_deref(),
        Path::new("/"),
        0,
    );
    if status.state != "enforced" {
        status.install_script = bundled.as_deref().and_then(|src| {
            let sha = sha256_hex(&std::fs::read(src).ok()?);
            install_script(
                &dropin,
                &bridge,
                &expected,
                src,
                &sha,
                cfg!(target_os = "macos"),
            )
        });
    }
    if status.state != "off" {
        status.uninstall_script = uninstall_script(&dropin, &bridge);
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("ac-firewall-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn settings() -> Value {
        dropin_settings(
            (2, 1, 330),
            Path::new("/etc/claude-code/managed-settings.d/50-agent-console.json"),
            Path::new("/opt/agent-console/hook-bridge"),
            Some(Path::new("/home/u")),
        )
    }

    #[test]
    fn the_dropin_locks_hooks_rules_bypass_and_sandbox() {
        let s = settings();
        assert_eq!(s["allowManagedHooksOnly"], true);
        assert_eq!(s["allowManagedPermissionRulesOnly"], true);
        assert_eq!(s["permissions"]["disableBypassPermissionsMode"], "disable");
        assert_eq!(s["sandbox"]["enabled"], true);
        assert_eq!(s["sandbox"]["allowUnsandboxedCommands"], false);
        assert_eq!(s["sandbox"]["failIfUnavailable"], true);
    }

    #[test]
    fn the_guard_and_the_approval_hook_block_on_failure_and_every_hook_is_the_root_owned_bridge() {
        let s = settings();
        let hooks = s["hooks"].as_object().unwrap();
        for (event, entries) in hooks {
            let hook = &entries[0]["hooks"][0];
            assert_eq!(entries[0]["matcher"], "*");
            let command = hook["command"].as_str().unwrap();
            assert!(
                command.starts_with("\"/opt/agent-console/hook-bridge\" "),
                "{event}"
            );
            match event.as_str() {
                // Every tool, before it runs: the guard, never the legacy
                // approvals bridge (which would answer in PermissionRequest's place).
                "PreToolUse" => {
                    assert_eq!(command, "\"/opt/agent-console/hook-bridge\" guard");
                    assert_eq!(hook["onFailure"], "block");
                    assert_eq!(hook["timeout"], 10);
                }
                "PermissionRequest" => {
                    assert_eq!(
                        command,
                        "\"/opt/agent-console/hook-bridge\" permissionrequest"
                    );
                    assert_eq!(hook["onFailure"], "block");
                }
                _ => assert!(hook.get("onFailure").is_none(), "{event} only records"),
            }
        }
        assert!(hooks.contains_key("PreToolUse"));
        assert!(hooks.contains_key("PermissionRequest"));
        assert!(hooks.contains_key("PostToolUse"));
    }

    #[test]
    fn the_console_state_and_the_managed_tier_are_denied_to_editing_tools() {
        let deny: Vec<String> = settings()["permissions"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(
            deny.contains(&"Edit(//etc/claude-code/**)".to_string()),
            "{deny:?}"
        );
        assert!(
            deny.contains(&"Edit(//opt/agent-console/**)".to_string()),
            "{deny:?}"
        );
        assert!(deny
            .iter()
            .all(|r| r.starts_with("Edit(") && r.ends_with("/**)")));
        assert_eq!(
            rule_path(
                Path::new("/home/u/.local/share/agent-console"),
                Some(Path::new("/home/u"))
            ),
            "~/.local/share/agent-console"
        );
    }

    #[test]
    fn an_event_the_cli_does_not_know_is_left_out() {
        let old = dropin_settings(
            (2, 0, 0),
            Path::new("/etc/x/d/50.json"),
            Path::new("/opt/x/hook-bridge"),
            None,
        );
        let hooks = old["hooks"].as_object().unwrap();
        assert!(hooks.contains_key("Stop"));
        assert!(!hooks.contains_key("PermissionRequest"));
    }

    #[test]
    fn the_install_script_carries_the_policy_and_checks_the_bridge_as_root() {
        let expected = render(&settings());
        let sha = "a".repeat(64);
        let script = install_script(
            Path::new("/etc/claude-code/managed-settings.d/50-agent-console.json"),
            Path::new("/opt/agent-console/hook-bridge"),
            &expected,
            Path::new("/home/it's me/app/hook-bridge"),
            &sha,
            false,
        )
        .unwrap();
        // The policy is in the text the user reads, byte for byte.
        assert!(script.contains(&format!(
            "<<'AGENT_CONSOLE_DROPIN'\n{expected}AGENT_CONSOLE_DROPIN\n"
        )));
        // The source is one quoted argument, quote and space included.
        assert!(script.contains(" '/home/it'\\''s me/app/hook-bridge' "));
        // Root-only first, verified, then executable, then in place.
        let order = [
            "-m 0700 ",
            &format!("echo '{sha}  /opt/agent-console/hook-bridge.new' | sudo sha256sum -c -"),
            "sudo chmod 0755 '/opt/agent-console/hook-bridge.new'",
            "sudo mv -f '/opt/agent-console/hook-bridge.new' '/opt/agent-console/hook-bridge'",
            "sudo chmod 0644 ",
            "sudo mv -f '/etc/claude-code/managed-settings.d/50-agent-console.json.new'",
        ];
        let mut at = 0;
        for step in order {
            let found = script[at..]
                .find(step)
                .unwrap_or_else(|| panic!("{step}\n{script}"));
            at += found + step.len();
        }
        assert!(script.starts_with("sh -eu <<'AGENT_CONSOLE_INSTALL'\n"));
        assert!(script.ends_with("\nAGENT_CONSOLE_INSTALL\n"));
        // It parses.
        #[cfg(unix)]
        {
            let ok = std::process::Command::new("sh")
                .arg("-n")
                .arg("-c")
                .arg(&script)
                .status()
                .unwrap();
            assert!(ok.success(), "{script}");
        }

        let mac = install_script(
            Path::new(
                "/Library/Application Support/ClaudeCode/managed-settings.d/50-agent-console.json",
            ),
            Path::new("/Library/Application Support/AgentConsole/hook-bridge"),
            &expected,
            Path::new("/Applications/Agent Console.app/Contents/MacOS/hook-bridge"),
            &sha,
            true,
        )
        .unwrap();
        assert!(mac.contains("| sudo shasum -a 256 -c -"));
        assert!(mac.contains("'/Library/Application Support/AgentConsole/hook-bridge'"));
    }

    #[test]
    fn no_script_for_a_path_or_a_hash_that_cannot_be_shown_honestly() {
        let expected = render(&settings());
        let dropin = Path::new("/etc/claude-code/managed-settings.d/50-agent-console.json");
        let bridge = Path::new("/opt/agent-console/hook-bridge");
        let sha = "a".repeat(64);
        // A line break in the source path.
        assert!(install_script(
            dropin,
            bridge,
            &expected,
            Path::new("/tmp/x\nsudo rm -rf /"),
            &sha,
            false
        )
        .is_none());
        // Not a sha256.
        assert!(install_script(
            dropin,
            bridge,
            &expected,
            Path::new("/tmp/x"),
            "$(id)",
            false
        )
        .is_none());
        // A policy that would close the here-document early.
        assert!(install_script(
            dropin,
            bridge,
            "{}\nAGENT_CONSOLE_DROPIN\nsudo id\n",
            Path::new("/tmp/x"),
            &sha,
            false
        )
        .is_none());
        assert_eq!(
            uninstall_script(dropin, bridge).unwrap(),
            "sudo rm -f '/etc/claude-code/managed-settings.d/50-agent-console.json' && sudo rm -f '/opt/agent-console/hook-bridge'\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn inspect_tells_off_enforced_drifted_and_unsafe_apart() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = temp();
        let me = std::fs::metadata(&root).unwrap().uid();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dir = root.join("managed-settings.d");
        let opt = root.join("opt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&opt).unwrap();
        for d in [&dir, &opt] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dropin = dir.join(DROPIN_NAME);
        let bridge = opt.join("hook-bridge");
        let bundled = root.join("bundled-bridge");
        std::fs::write(&bundled, b"bridge v2").unwrap();
        let expected = render(&settings());
        let look = |uid| inspect(&dropin, &bridge, &expected, Some(&bundled), &root, uid);

        assert_eq!(look(me).state, "off");

        std::fs::write(&dropin, &expected).unwrap();
        std::fs::set_permissions(&dropin, std::fs::Permissions::from_mode(0o644)).unwrap();
        let s = look(me);
        assert_eq!(s.state, "drifted", "{:?}", s.problems);
        assert!(s.problems[0].contains("missing"));

        std::fs::write(&bridge, b"bridge v2").unwrap();
        std::fs::set_permissions(&bridge, std::fs::Permissions::from_mode(0o755)).unwrap();
        let s = look(me);
        assert_eq!(s.state, "enforced", "{:?}", s.problems);
        assert!(s.engines.iter().any(|e| e.engine == "claude" && e.covered));
        assert!(s.engines.iter().all(|e| e.engine == "claude" || !e.covered));

        // The same files owned by someone who is not the administrator.
        let s = look(me.wrapping_add(1));
        assert_eq!(s.state, "unsafe");
        assert!(s.engines.iter().all(|e| !e.covered));

        // An old bridge after an update of the console.
        std::fs::write(&bridge, b"bridge v1").unwrap();
        assert_eq!(look(me).state, "drifted");
        std::fs::write(&bridge, b"bridge v2").unwrap();

        // One byte of the policy changed.
        std::fs::write(&dropin, expected.replace("\"disable\"", "\"allow\"")).unwrap();
        assert_eq!(look(me).state, "drifted");
        std::fs::write(&dropin, &expected).unwrap();

        // A directory on the way that others can write: the file can be swapped.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(look(me).state, "unsafe");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        // Unreadable by others: Claude Code would start without it.
        std::fs::set_permissions(&dropin, std::fs::Permissions::from_mode(0o600)).unwrap();
        let s = look(me);
        assert_eq!(s.state, "unsafe");
        assert!(s.problems.iter().any(|p| p.contains("not readable")));
        std::fs::set_permissions(&dropin, std::fs::Permissions::from_mode(0o644)).unwrap();

        // A symlink in place of the drop-in.
        let real = root.join("elsewhere.json");
        std::fs::write(&real, &expected).unwrap();
        std::fs::remove_file(&dropin).unwrap();
        std::os::unix::fs::symlink(&real, &dropin).unwrap();
        assert_eq!(look(me).state, "unsafe");

        let _ = std::fs::remove_dir_all(&root);
    }
}
