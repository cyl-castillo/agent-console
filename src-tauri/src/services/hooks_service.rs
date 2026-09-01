use parking_lot::Mutex;
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};
use crate::services::activity_service::ActivityEvent;
use crate::services::snapshot_service;
use crate::state::AppState;

/// Bundled hook scripts written to disk at runtime.
const USERPROMPT_HOOK: &str = include_str!("../../resources/userprompt-hook.cjs");
const PRETOOLUSE_HOOK: &str = include_str!("../../resources/pretooluse-hook.cjs");
const STOP_HOOK: &str = include_str!("../../resources/stop-hook.cjs");
const POSTTOOLUSE_HOOK: &str = include_str!("../../resources/posttooluse-hook.cjs");
const MODELSWITCH_HOOK: &str = include_str!("../../resources/modelswitch-hook.cjs");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksStatus {
    pub session_dir: PathBuf,
    pub script_path: PathBuf,
    pub pretooluse_script_path: PathBuf,
    pub installed: bool,
    pub pretooluse_installed: bool,
    pub posttooluse_installed: bool,
    pub settings_path: PathBuf,
    /// Codex mirror (same bridge scripts, wired via ~/.codex/hooks.json).
    /// codex_available=false ⇒ the codex CLI isn't installed; the other two
    /// then just report the file state.
    pub codex_available: bool,
    pub codex_installed: bool,
    pub codex_pretooluse_installed: bool,
    pub codex_hooks_path: PathBuf,
}

pub struct HooksRuntime {
    session_dir: PathBuf,
    binary_path: Option<PathBuf>,
    script_path: PathBuf,
    pretooluse_script_path: PathBuf,
    stop_script_path: PathBuf,
    posttooluse_script_path: PathBuf,
    /// Claude-only (PostModelSwitch, 2.1.251+) — never mirrored into Codex.
    modelswitch_script_path: PathBuf,
    watcher_started: Mutex<bool>,
    approvals_watcher_started: Mutex<bool>,
}

/// Session dirs are keyed by PID and never reused, so every app run leaves one
/// behind (events.jsonl + approvals/). Without GC the cache grows forever —
/// a real install accumulated 200+ dirs in two weeks. 30 days is comfortably
/// past any live process while keeping recent dirs for debugging.
const SESSION_DIR_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Remove per-PID session dirs older than `cutoff` (by mtime), keeping
/// `keep` (the current process's dir). Best-effort: a dir that fails to
/// delete is skipped, never an error. Returns how many were removed.
fn prune_stale_session_dirs(
    sessions_root: &Path,
    cutoff: std::time::SystemTime,
    keep: &Path,
) -> usize {
    let Ok(entries) = fs::read_dir(sessions_root) else {
        return 0;
    };
    let mut removed = 0;
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() || path == keep {
            continue;
        }
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t < cutoff)
            .unwrap_or(false);
        if stale && fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

impl HooksRuntime {
    pub fn new() -> AppResult<Self> {
        let cache = dirs::cache_dir()
            .ok_or_else(|| AppError::Other("no cache dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&cache)?;
        let sessions_root = cache.join("sessions");
        let session_dir = sessions_root.join(format!("{}", std::process::id()));
        fs::create_dir_all(&session_dir)?;
        // GC on startup: old per-PID dirs can't belong to a live process.
        if let Some(cutoff) = std::time::SystemTime::now().checked_sub(SESSION_DIR_MAX_AGE) {
            let n = prune_stale_session_dirs(&sessions_root, cutoff, &session_dir);
            if n > 0 {
                eprintln!("hooks: pruned {n} stale session dirs");
            }
        }
        fs::create_dir_all(session_dir.join("approvals"))?;
        let script_path = ensure_hook_script(&cache, "userprompt-hook.cjs", USERPROMPT_HOOK)?;
        let pretooluse_script_path =
            ensure_hook_script(&cache, "pretooluse-hook.cjs", PRETOOLUSE_HOOK)?;
        let stop_script_path = ensure_hook_script(&cache, "stop-hook.cjs", STOP_HOOK)?;
        let posttooluse_script_path =
            ensure_hook_script(&cache, "posttooluse-hook.cjs", POSTTOOLUSE_HOOK)?;
        let modelswitch_script_path =
            ensure_hook_script(&cache, "modelswitch-hook.cjs", MODELSWITCH_HOOK)?;
        let binary_path = ensure_hook_binary(&cache);
        Ok(Self {
            session_dir,
            binary_path,
            script_path,
            pretooluse_script_path,
            stop_script_path,
            posttooluse_script_path,
            modelswitch_script_path,
            watcher_started: Mutex::new(false),
            approvals_watcher_started: Mutex::new(false),
        })
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }
    /// Stable cache-dir copy of the native bridge binary, when the sidecar
    /// shipped with this build. None ⇒ hook commands fall back to node.
    #[allow(dead_code)]
    pub fn bridge_binary(&self) -> Option<&Path> {
        self.binary_path.as_deref()
    }
    #[allow(dead_code)]
    pub fn script_path(&self) -> &Path {
        &self.script_path
    }

    /// Start the events.jsonl watcher (idempotent).
    pub fn start_watcher(&self, app: AppHandle) {
        let mut started = self.watcher_started.lock();
        if !*started {
            *started = true;
            let dir = self.session_dir.clone();
            let app2 = app.clone();
            thread::spawn(move || watcher_loop(app2, dir));
        }
        drop(started);

        let mut started2 = self.approvals_watcher_started.lock();
        if !*started2 {
            *started2 = true;
            let dir = self.session_dir.join("approvals");
            thread::spawn(move || approvals_watcher_loop(app, dir));
        }
    }

    pub fn status(&self) -> HooksStatus {
        let settings_path = settings_path();
        let installed = is_hook_installed(&settings_path, "UserPromptSubmit", &self.script_path)
            .unwrap_or(false);
        let pretooluse_installed =
            is_hook_installed(&settings_path, "PreToolUse", &self.pretooluse_script_path)
                .unwrap_or(false);
        let posttooluse_installed =
            is_hook_installed(&settings_path, "PostToolUse", &self.posttooluse_script_path)
                .unwrap_or(false);
        let codex_hooks_path = codex_hooks_path();
        let codex_installed =
            is_hook_installed(&codex_hooks_path, "UserPromptSubmit", &self.script_path)
                .unwrap_or(false);
        let codex_pretooluse_installed = is_hook_installed(
            &codex_hooks_path,
            "PreToolUse",
            &self.pretooluse_script_path,
        )
        .unwrap_or(false);
        HooksStatus {
            session_dir: self.session_dir.clone(),
            script_path: self.script_path.clone(),
            pretooluse_script_path: self.pretooluse_script_path.clone(),
            installed,
            pretooluse_installed,
            posttooluse_installed,
            settings_path,
            codex_available: codex_available(),
            codex_installed,
            codex_pretooluse_installed,
            codex_hooks_path,
        }
    }

    /// Merge UserPromptSubmit + PreToolUse hooks into ~/.claude/settings.json —
    /// and, when the codex CLI is present, into ~/.codex/hooks.json too (Codex
    /// adopted the same hooks table shape and PreToolUse decision schema, so the
    /// same bridge scripts serve both engines).
    pub fn install(&self) -> AppResult<HooksStatus> {
        let settings_path = settings_path();
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut settings: Value = if settings_path.exists() {
            let txt = fs::read_to_string(&settings_path)?;
            serde_json::from_str(&txt).unwrap_or(json!({}))
        } else {
            json!({})
        };
        if !settings.is_object() {
            settings = json!({});
        }

        upsert_hook(
            &mut settings,
            "UserPromptSubmit",
            &self.script_path,
            self.binary_path.as_deref(),
        );
        upsert_hook(
            &mut settings,
            "PreToolUse",
            &self.pretooluse_script_path,
            self.binary_path.as_deref(),
        );
        upsert_hook(
            &mut settings,
            "Stop",
            &self.stop_script_path,
            self.binary_path.as_deref(),
        );
        upsert_hook(
            &mut settings,
            "PostToolUse",
            &self.posttooluse_script_path,
            self.binary_path.as_deref(),
        );
        // Claude-only: Codex has no model-switch event, so this one is absent
        // from the mirror list below on purpose.
        upsert_hook(
            &mut settings,
            "PostModelSwitch",
            &self.modelswitch_script_path,
            self.binary_path.as_deref(),
        );

        write_settings_atomic(&settings_path, &settings)?;

        if codex_available() {
            self.install_codex(&[
                ("UserPromptSubmit", &self.script_path),
                ("PreToolUse", &self.pretooluse_script_path),
                ("Stop", &self.stop_script_path),
                ("PostToolUse", &self.posttooluse_script_path),
            ])?;
        }
        Ok(self.status())
    }

    /// Merge the given hook entries into ~/.codex/hooks.json (created if
    /// missing; other content preserved; idempotent).
    fn install_codex(&self, hooks: &[(&str, &PathBuf)]) -> AppResult<()> {
        merge_hooks_into(&codex_hooks_path(), hooks, self.binary_path.as_deref())
    }

    /// Mirror into ~/.codex/hooks.json every hook event that is already
    /// installed on the Claude side but missing on the Codex side. Closes the
    /// install-order gap: enable the approvals bridge first, install the codex
    /// CLI later — install() only wired codex if it was present at that
    /// instant, so the UI said "bridge active" while codex approvals stayed in
    /// the terminal. Runs on every startup (cheap when codex is absent or
    /// nothing is missing). Mirroring only what's claude-installed keeps
    /// PreToolUse opt-in, and uninstall() clears both sides so a deliberate
    /// uninstall never gets resurrected here.
    pub fn sync_codex_hooks(&self) -> AppResult<()> {
        if !codex_available() {
            return Ok(());
        }
        let pairs: [(&str, &PathBuf); 4] = [
            ("UserPromptSubmit", &self.script_path),
            ("PreToolUse", &self.pretooluse_script_path),
            ("Stop", &self.stop_script_path),
            ("PostToolUse", &self.posttooluse_script_path),
        ];
        let n = sync_hooks_between(
            &settings_path(),
            &codex_hooks_path(),
            &pairs,
            self.binary_path.as_deref(),
        )?;
        if n > 0 {
            eprintln!("hooks: mirrored {n} claude hook(s) into codex");
        }
        Ok(())
    }

    /// Auto-install the lightweight UserPromptSubmit observer on first run, so
    /// the features that depend on it (session-name suggestions, resume binding,
    /// activity stream, auto-snapshots) work out of the box — not only after the
    /// user finds and flips the integration toggle. This was the gap behind
    /// "the name recommender only works on my machine": on a clean install the
    /// hook was never registered.
    ///
    /// Guarded by a one-time marker so a deliberate `uninstall()` sticks across
    /// restarts (we never re-add it). The PreToolUse permission bridge is NOT
    /// auto-installed — it changes how Claude runs and stays opt-in. Snapshots
    /// are safe to enable by default: they're written to a private ref via
    /// write-tree/commit-tree and never touch HEAD, the index, or the worktree.
    pub fn ensure_autoinstalled(&self) -> AppResult<()> {
        let marker = self.script_path.with_file_name(".userprompt-autoinstalled");
        if marker.exists() {
            return Ok(());
        }
        let settings_path = settings_path();
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut settings: Value = if settings_path.exists() {
            serde_json::from_str(&fs::read_to_string(&settings_path)?).unwrap_or(json!({}))
        } else {
            json!({})
        };
        if !settings.is_object() {
            settings = json!({});
        }
        upsert_hook(
            &mut settings,
            "UserPromptSubmit",
            &self.script_path,
            self.binary_path.as_deref(),
        );
        write_settings_atomic(&settings_path, &settings)?;
        // Best-effort marker: if it fails we'd re-run the idempotent upsert next
        // launch, which is harmless.
        let _ = fs::write(&marker, b"1");
        Ok(())
    }

    /// Codex twin of `ensure_autoinstalled`, with its OWN marker: existing
    /// installs already carry the claude marker, and without a separate one the
    /// codex observer would never get wired for them. Only runs when the codex
    /// CLI is present (an installed hook triggers codex's one-time trust
    /// prompt — noise for people who never use codex). Observer only; the
    /// PreToolUse bridge stays opt-in, exactly like Claude's.
    pub fn ensure_codex_autoinstalled(&self) -> AppResult<()> {
        let marker = self
            .script_path
            .with_file_name(".userprompt-codex-autoinstalled");
        if marker.exists() || !codex_available() {
            return Ok(());
        }
        self.install_codex(&[("UserPromptSubmit", &self.script_path)])?;
        let _ = fs::write(&marker, b"1");
        Ok(())
    }

    /// Auto-install the Stop (turn-completed) observer for both engines, under
    /// its OWN marker — the userprompt markers already exist on installed
    /// machines, so bundling Stop with them would never reach existing users.
    /// Observer-class like the prompt hook (writes an event, changes no
    /// behavior), so it's safe to enable by default.
    pub fn ensure_stop_autoinstalled(&self) -> AppResult<()> {
        let marker = self.script_path.with_file_name(".stop-autoinstalled");
        if marker.exists() {
            return Ok(());
        }
        let settings_path = settings_path();
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut settings: Value = if settings_path.exists() {
            serde_json::from_str(&fs::read_to_string(&settings_path)?).unwrap_or(json!({}))
        } else {
            json!({})
        };
        if !settings.is_object() {
            settings = json!({});
        }
        upsert_hook(
            &mut settings,
            "Stop",
            &self.stop_script_path,
            self.binary_path.as_deref(),
        );
        write_settings_atomic(&settings_path, &settings)?;
        if codex_available() {
            self.install_codex(&[("Stop", &self.stop_script_path)])?;
        }
        let _ = fs::write(&marker, b"1");
        Ok(())
    }

    /// Auto-install the PostToolUse (tool-result) observer for both engines,
    /// under its OWN marker (same reasoning as Stop: existing installs carry
    /// the older markers and would never get it otherwise). Observer-class —
    /// it records what tools produced and changes no behavior — so it's safe
    /// to enable by default. Feeds the Testigo turn evidence.
    pub fn ensure_posttooluse_autoinstalled(&self) -> AppResult<()> {
        let marker = self
            .script_path
            .with_file_name(".posttooluse-autoinstalled");
        if marker.exists() {
            return Ok(());
        }
        let settings_path = settings_path();
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut settings: Value = if settings_path.exists() {
            serde_json::from_str(&fs::read_to_string(&settings_path)?).unwrap_or(json!({}))
        } else {
            json!({})
        };
        if !settings.is_object() {
            settings = json!({});
        }
        upsert_hook(
            &mut settings,
            "PostToolUse",
            &self.posttooluse_script_path,
            self.binary_path.as_deref(),
        );
        write_settings_atomic(&settings_path, &settings)?;
        if codex_available() {
            self.install_codex(&[("PostToolUse", &self.posttooluse_script_path)])?;
        }
        let _ = fs::write(&marker, b"1");
        Ok(())
    }

    /// Auto-install the PostModelSwitch observer, under its OWN marker (same
    /// rollout pattern as Stop/PostToolUse). Claude-only: the event is Claude's
    /// (2.1.251+) and Codex has no equivalent, so nothing is mirrored into
    /// ~/.codex/hooks.json — installing an event Codex doesn't know would only
    /// add a dead entry to the user's file.
    ///
    /// Observer-class and then some: PostModelSwitch is an ASYNC event, so the
    /// CLI doesn't even read our decision fields — the switch has already
    /// happened. Older Claude versions simply never fire an event they don't
    /// have, which is exactly how they degrade: the pill keeps showing intent,
    /// as it did before.
    pub fn ensure_modelswitch_autoinstalled(&self) -> AppResult<()> {
        let marker = self
            .script_path
            .with_file_name(".modelswitch-autoinstalled");
        if marker.exists() {
            return Ok(());
        }
        let settings_path = settings_path();
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut settings: Value = if settings_path.exists() {
            serde_json::from_str(&fs::read_to_string(&settings_path)?).unwrap_or(json!({}))
        } else {
            json!({})
        };
        if !settings.is_object() {
            settings = json!({});
        }
        upsert_hook(
            &mut settings,
            "PostModelSwitch",
            &self.modelswitch_script_path,
            self.binary_path.as_deref(),
        );
        write_settings_atomic(&settings_path, &settings)?;
        let _ = fs::write(&marker, b"1");
        Ok(())
    }

    /// Migrate any of OUR already-installed hook entries from the legacy
    /// bare-path command format to the canonical `node "<path>"` form, in both
    /// Claude's settings.json and Codex's hooks.json. Runs on every startup
    /// (cheap: read + compare; writes only when something changed) because the
    /// one-time autoinstall markers mean existing installs never re-run
    /// upsert_hook. Only rewrites events where the script is ALREADY
    /// registered — a deliberate uninstall stays uninstalled.
    pub fn normalize_hook_commands(&self) -> AppResult<()> {
        let pairs: [(&str, &PathBuf); 5] = [
            ("UserPromptSubmit", &self.script_path),
            ("PreToolUse", &self.pretooluse_script_path),
            ("Stop", &self.stop_script_path),
            ("PostToolUse", &self.posttooluse_script_path),
            ("PostModelSwitch", &self.modelswitch_script_path),
        ];
        for path in [settings_path(), codex_hooks_path()] {
            if !path.exists() {
                continue;
            }
            let txt = fs::read_to_string(&path)?;
            let mut settings: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
            if !settings.is_object() {
                continue;
            }
            let before = settings.to_string();
            for (event, script) in pairs {
                let installed = settings
                    .pointer(&format!("/hooks/{event}"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter().any(|e| {
                            has_command_path(e, script) || has_bridge_command(e, bridge_mode(event))
                        })
                    })
                    .unwrap_or(false);
                if installed {
                    upsert_hook(&mut settings, event, script, self.binary_path.as_deref());
                }
            }
            let after = settings.to_string();
            if after != before {
                write_settings_atomic(&path, &settings)?;
            }
        }
        Ok(())
    }

    /// Remove our hook entries from settings.json and ~/.codex/hooks.json.
    /// Other hooks/settings untouched.
    pub fn uninstall(&self) -> AppResult<HooksStatus> {
        for path in [settings_path(), codex_hooks_path()] {
            if !path.exists() {
                continue;
            }
            let txt = fs::read_to_string(&path)?;
            let mut settings: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
            if let Some(hooks) = settings.get_mut("hooks").and_then(|v| v.as_object_mut()) {
                for (key, target) in [
                    ("UserPromptSubmit", &self.script_path),
                    ("PreToolUse", &self.pretooluse_script_path),
                    ("Stop", &self.stop_script_path),
                    ("PostToolUse", &self.posttooluse_script_path),
                    // Claude-only, but listed for both paths: removing an event
                    // Codex never had is a no-op, and skipping it here would
                    // strand the entry if a future Codex ever grows one.
                    ("PostModelSwitch", &self.modelswitch_script_path),
                ] {
                    if let Some(arr) = hooks.get_mut(key).and_then(|v| v.as_array_mut()) {
                        arr.retain(|e| {
                            !has_command_path(e, target) && !has_bridge_command(e, bridge_mode(key))
                        });
                    }
                }
            }
            write_settings_atomic(&path, &settings)?;
        }
        Ok(self.status())
    }

    /// Approval requests currently pending on disk (<session_dir>/approvals/
    /// *.req.json — the hook deletes each file once decided or timed out).
    /// The frontend resyncs its queue from this on attach and window focus, so
    /// a missed `approval://request` event (webview reload, HMR, listener not
    /// yet attached) degrades to a short delay instead of an invisible 90s
    /// stall on the agent.
    pub fn pending_approvals(&self) -> Vec<Value> {
        let dir = self.session_dir.join("approvals");
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut reqs: Vec<Value> = entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".req.json"))
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|txt| serde_json::from_str::<Value>(&txt).ok())
            .collect();
        reqs.sort_by_key(|v| v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0));
        reqs
    }

    /// Write a response for an in-flight approval. The hook script is polling
    /// for <session_dir>/approvals/<id>.res.json and will pick this up.
    pub fn respond(&self, id: &str, decision: &str, reason: Option<&str>) -> AppResult<()> {
        if !["allow", "deny", "ask"].contains(&decision) {
            return Err(AppError::InvalidArgument(format!(
                "bad decision: {decision}"
            )));
        }
        let dir = self.session_dir.join("approvals");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{id}.res.json"));
        let body = json!({ "id": id, "decision": decision, "reason": reason });
        // Write to a temp file then rename, so the polling hook never reads a partial JSON.
        let tmp = dir.join(format!("{id}.res.tmp"));
        fs::write(&tmp, body.to_string())?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Merge hook entries into a hooks-table JSON file (created if missing; other
/// content preserved; idempotent).
fn merge_hooks_into(
    path: &Path,
    hooks: &[(&str, &PathBuf)],
    binary: Option<&Path>,
) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut root: Value = if path.exists() {
        serde_json::from_str(&fs::read_to_string(path)?).unwrap_or(json!({}))
    } else {
        json!({})
    };
    if !root.is_object() {
        root = json!({});
    }
    for (event, script) in hooks {
        upsert_hook(&mut root, event, script, binary);
    }
    write_settings_atomic(path, &root)
}

/// Copy every hook event from `pairs` that is installed in `claude_path` but
/// missing in `codex_path`. Returns how many events were mirrored (0 ⇒ no
/// write at all). Free function over explicit paths so it's unit-testable
/// without touching the real home dir.
fn sync_hooks_between(
    claude_path: &Path,
    codex_path: &Path,
    pairs: &[(&str, &PathBuf)],
    binary: Option<&Path>,
) -> AppResult<usize> {
    let missing: Vec<(&str, &PathBuf)> = pairs
        .iter()
        .filter(|(event, script)| {
            is_hook_installed(claude_path, event, script).unwrap_or(false)
                && !is_hook_installed(codex_path, event, script).unwrap_or(false)
        })
        .copied()
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }
    merge_hooks_into(codex_path, &missing, binary)?;
    Ok(missing.len())
}

/// Write settings.json via temp file + rename so a crash mid-write can never
/// leave ~/.claude/settings.json truncated or half-serialized.
fn write_settings_atomic(settings_path: &Path, settings: &Value) -> AppResult<()> {
    let tmp = settings_path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(settings).unwrap())?;
    fs::rename(&tmp, settings_path)?;
    Ok(())
}

/// Legacy hook command: `node "<script>"`.
///
/// The oldest format was the bare script path, which relied on shebang
/// execution. That works on Unix but is dead on Windows twice over: cmd.exe
/// has no shebangs and `.cjs` has no default file association, and the
/// unquoted path split at the first space in the user's home dir
/// ("C:\Users\Melissa Mujica\…"). So on Windows NO hook ever fired. Quoted
/// path + explicit interpreter fixed that — but still required node.
fn hook_command(script_path: &Path) -> String {
    format!("node \"{}\"", script_path.to_string_lossy())
}

/// Canonical hook command, third generation: `"<hook-bridge>" <mode>` — the
/// native bridge binary, no node required. Falls back to the node form when
/// the binary isn't available (dev runs without the sidecar, copy failure),
/// so hooks keep working exactly as before instead of silently dying.
fn bridge_command(binary: Option<&Path>, script_path: &Path, mode: &str) -> String {
    match binary {
        Some(b) => format!("\"{}\" {mode}", b.to_string_lossy()),
        None => hook_command(script_path),
    }
}

/// Does this entry carry OUR native-bridge command for `mode`? Matched by the
/// binary's filename plus the trailing mode word, so entries survive cache
/// relocations (same reasoning as the path-containment match for scripts).
fn has_bridge_command(entry: &Value, mode: &str) -> bool {
    let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) else {
        return false;
    };
    hooks.iter().any(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .map(|c| c.contains(BRIDGE_BIN_NAME) && c.trim_end().ends_with(&format!("\" {mode}")))
            .unwrap_or(false)
    })
}

fn entry_has_command(entry: &Value, cmd: &str) -> bool {
    entry
        .get("hooks")
        .and_then(|v| v.as_array())
        .map(|hs| {
            hs.iter()
                .any(|h| h.get("command").and_then(|c| c.as_str()) == Some(cmd))
        })
        .unwrap_or(false)
}

fn upsert_hook(settings: &mut Value, event: &str, script_path: &Path, binary: Option<&Path>) {
    let mode = bridge_mode(event);
    let canonical = bridge_command(binary, script_path, mode);
    let hooks = settings.get("hooks").cloned().unwrap_or(json!({}));
    let mut hooks = if hooks.is_object() { hooks } else { json!({}) };
    let mut arr = hooks
        .get_mut(event)
        .and_then(|v| v.as_array_mut())
        .cloned()
        .unwrap_or_default();
    if !arr.iter().any(|e| entry_has_command(e, &canonical)) {
        // Drop any prior entry of ours — legacy bare path, node form, or a
        // bridge command pointing at an old location — so a re-install
        // migrates it instead of duplicating the hook.
        arr.retain(|e| !has_command_path(e, script_path) && !has_bridge_command(e, mode));
        arr.push(json!({
            "matcher": "*",
            "hooks": [{ "type": "command", "command": canonical }]
        }));
    }
    hooks
        .as_object_mut()
        .unwrap()
        .insert(event.to_string(), Value::Array(arr));
    settings
        .as_object_mut()
        .unwrap()
        .insert("hooks".to_string(), hooks);
}

fn settings_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".claude/settings.json"))
        .unwrap_or_else(|| PathBuf::from(".claude/settings.json"))
}

/// Codex reads hooks from ~/.codex/hooks.json — same top-level `hooks` table
/// shape as Claude's settings.json, so the upsert/lookup helpers are shared.
fn codex_hooks_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".codex/hooks.json"))
        .unwrap_or_else(|| PathBuf::from(".codex/hooks.json"))
}

/// Only wire Codex hooks when the codex CLI is actually present: an installed
/// hook makes codex show a one-time trust prompt, which would be spooky noise
/// for users who never use codex.
fn codex_available() -> bool {
    crate::services::claude_cli::codex_command_with_stdin(&["--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The native bridge binary's filename (with .exe on Windows).
#[cfg(windows)]
const BRIDGE_BIN_NAME: &str = "hook-bridge.exe";
#[cfg(not(windows))]
const BRIDGE_BIN_NAME: &str = "hook-bridge";

/// Hook event → bridge binary mode argument.
fn bridge_mode(event: &str) -> &'static str {
    match event {
        "UserPromptSubmit" => "userprompt",
        "PreToolUse" => "pretooluse",
        "PostToolUse" => "posttooluse",
        "PostModelSwitch" => "modelswitch",
        _ => "stop",
    }
}

/// Copy the bundled hook-bridge sidecar to a STABLE path under the cache dir.
/// The sidecar ships next to the app executable, but that location moves on
/// every AppImage launch (fresh /tmp mount) and on every update — a hooks.json
/// pointing there would go stale immediately. The cache copy is the address
/// the hook entries reference; refreshed on every start like the scripts.
/// None ⇒ no sidecar next to the exe (dev run without it) — callers fall back
/// to the node scripts.
fn ensure_hook_binary(runtime_dir: &Path) -> Option<PathBuf> {
    let sidecar = std::env::current_exe()
        .ok()?
        .parent()?
        .join(BRIDGE_BIN_NAME);
    if !sidecar.is_file() {
        return None;
    }
    let bin_dir = runtime_dir.join("bin");
    fs::create_dir_all(&bin_dir).ok()?;
    let dest = bin_dir.join(BRIDGE_BIN_NAME);
    // Copy through a temp name + rename so a hook firing mid-update never
    // execs a half-written binary.
    let tmp = bin_dir.join(format!("{BRIDGE_BIN_NAME}.tmp"));
    fs::copy(&sidecar, &tmp).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&tmp).ok()?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&tmp, perm).ok()?;
    }
    fs::rename(&tmp, &dest).ok()?;
    Some(dest)
}

fn ensure_hook_script(runtime_dir: &Path, filename: &str, body: &str) -> AppResult<PathBuf> {
    let path = runtime_dir.join(filename);
    fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&path)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&path, perm)?;
    }
    Ok(path)
}

fn is_hook_installed(settings_path: &Path, event: &str, script_path: &Path) -> AppResult<bool> {
    if !settings_path.exists() {
        return Ok(false);
    }
    let txt = fs::read_to_string(settings_path)?;
    let v: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
    let Some(arr) = v
        .pointer(&format!("/hooks/{event}"))
        .and_then(|v| v.as_array())
    else {
        return Ok(false);
    };
    Ok(arr
        .iter()
        .any(|e| has_command_path(e, script_path) || has_bridge_command(e, bridge_mode(event))))
}

/// Does this settings entry point at our script? Matches BOTH command formats:
/// the legacy bare path and the canonical `node "<path>"` — install stays
/// idempotent and uninstall removes either generation.
fn has_command_path(entry: &Value, target: &Path) -> bool {
    let target_str = target.to_string_lossy().to_string();
    let quoted = format!("\"{target_str}\"");
    let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) else {
        return false;
    };
    hooks.iter().any(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .map(|c| c == target_str || c.contains(&quoted))
            .unwrap_or(false)
    })
}

/// Watch a directory and return (watcher, wake-receiver, wait-interval).
///
/// Preferred mode: a platform filesystem watcher wakes the loop the moment
/// something changes, with a slow 2s heartbeat as a safety net for missed
/// events. If the watcher can't start (inotify exhaustion, exotic FS), fall
/// back to pure polling at `poll_fallback` — the pre-notify behavior.
fn dir_wake_source(
    dir: &Path,
    poll_fallback: Duration,
) -> (
    Option<notify::RecommendedWatcher>,
    mpsc::Receiver<()>,
    Duration,
) {
    let (tx, rx) = mpsc::channel::<()>();
    let watcher = notify::recommended_watcher(move |_res| {
        let _ = tx.send(());
    })
    .ok()
    .and_then(|mut w| match w.watch(dir, RecursiveMode::NonRecursive) {
        Ok(()) => Some(w),
        Err(e) => {
            eprintln!("hooks: fs watch on {} failed, polling: {e}", dir.display());
            None
        }
    });
    let interval = if watcher.is_some() {
        Duration::from_secs(2)
    } else {
        poll_fallback
    };
    (watcher, rx, interval)
}

/// Block until the next filesystem wake-up (or the heartbeat), then coalesce
/// any burst of queued wake-ups into this one pass.
fn await_wake(rx: &mpsc::Receiver<()>, interval: Duration, watching: bool) {
    if watching {
        let _ = rx.recv_timeout(interval);
        while rx.try_recv().is_ok() {}
    } else {
        thread::sleep(interval);
    }
}

/// Parse JSONL appended to `path` past `last_size`. Returns the events and the
/// new read offset. Only complete lines (through the last `\n`) are consumed —
/// a torn tail mid-append stays buffered for the next pass instead of being
/// half-parsed and dropped. A shrunken file (truncation) resets to 0 so the
/// next pass reprocesses from the start.
fn drain_new_lines(path: &Path, last_size: u64) -> (Vec<Value>, u64) {
    let Ok(meta) = fs::metadata(path) else {
        return (Vec::new(), last_size);
    };
    let size = meta.len();
    if size == last_size {
        return (Vec::new(), last_size);
    }
    if size < last_size {
        return (Vec::new(), 0);
    }
    let Ok(mut f) = fs::File::open(path) else {
        return (Vec::new(), last_size);
    };
    if f.seek(SeekFrom::Start(last_size)).is_err() {
        return (Vec::new(), last_size);
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return (Vec::new(), last_size);
    }
    let Some(consumed) = buf.rfind('\n').map(|i| i + 1) else {
        return (Vec::new(), last_size);
    };
    // Cap per-event size BEFORE parsing: a buggy (or hostile) hook writing a
    // multi-MB line must not balloon the app's memory — the ledger bounds its
    // payloads anyway, so an oversized event carries nothing we'd keep.
    const MAX_EVENT_BYTES: usize = 1024 * 1024;
    let events = buf[..consumed]
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            let ok = l.len() <= MAX_EVENT_BYTES;
            if !ok {
                eprintln!("hooks: dropping oversized event line ({} bytes)", l.len());
            }
            ok
        })
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    (events, last_size + consumed as u64)
}

fn watcher_loop(app: AppHandle, dir: PathBuf) {
    let events_file = dir.join("events.jsonl");
    let (watcher, rx, interval) = dir_wake_source(&dir, Duration::from_millis(120));
    let mut last_size: u64 = 0;
    loop {
        await_wake(&rx, interval, watcher.is_some());
        let (events, next) = drain_new_lines(&events_file, last_size);
        last_size = next;
        for v in &events {
            handle_event(v, &app);
        }
    }
}

fn handle_event(v: &Value, app: &AppHandle) {
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let _ = app.emit(&format!("hook://{kind}"), v);

    if kind == "user_prompt" {
        let state = app.state::<AppState>();
        let project = state.inner.lock().project.clone();
        if let Some(p) = project {
            // Persist the prompt to the durable per-project ledger that
            // "learning mode" reflects over. Best-effort: a failed append must
            // never block the snapshot or the UI event.
            let root = p.root.to_string_lossy();
            let ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
            // Best-effort but never SILENT: a failed persistence append (disk
            // full, permissions) is data loss the user can't see happening —
            // log it so "why is my history empty" has a trail (R3 policy).
            if let Err(e) = state.activity.record(
                root.as_ref(),
                &ActivityEvent {
                    ts,
                    kind: "user_prompt".into(),
                    prompt: str_field(v, "prompt"),
                    skill: str_field(v, "skill"),
                    term_id: str_field(v, "termId"),
                    session_id: str_field(v, "sessionId"),
                    snapshot_sha: None,
                },
            ) {
                eprintln!("hooks: activity append failed: {e}");
            }

            // Testigo: the prompt is the intent — it opens a turn in the
            // evidence chain. Best-effort like the activity append. A
            // witness-off project errs here BY DESIGN (that's the per-project
            // opt-out working) — only real failures get logged.
            if let Err(e) = state.testigo.on_prompt(
                root.as_ref(),
                ts,
                str_field(v, "termId").as_deref(),
                str_field(v, "sessionId").as_deref(),
                str_field(v, "prompt").as_deref(),
                str_field(v, "skill").as_deref(),
                str_field(v, "cwd").as_deref(),
            ) {
                let msg = e.to_string();
                if !msg.contains("witnessing disabled") {
                    eprintln!("hooks: testigo append failed: {msg}");
                }
            }

            // Auto-snapshot the working tree the prompt ran in. Worktree
            // sessions report their own cwd — snapshotting the project root
            // there would checkpoint the wrong checkout. Fall back to the
            // project root for events without a cwd (older hook script).
            let snap_repo = str_field(v, "cwd")
                .map(PathBuf::from)
                .filter(|c| c.is_dir())
                .unwrap_or_else(|| p.root.clone());
            let id = uuid::Uuid::new_v4().to_string();
            if let Ok(Some(snap)) = snapshot_service::create(&snap_repo, &id) {
                // Record the snapshot too, so reflection can correlate a prompt
                // with the working-tree checkpoint it produced.
                let _ = state.activity.record(
                    root.as_ref(),
                    &ActivityEvent {
                        ts,
                        kind: "snapshot".into(),
                        prompt: None,
                        skill: None,
                        term_id: str_field(v, "termId"),
                        session_id: str_field(v, "sessionId"),
                        snapshot_sha: Some(snap.commit_sha.clone()),
                    },
                );
                let _ = state.testigo.on_snapshot(
                    root.as_ref(),
                    ts,
                    str_field(v, "termId").as_deref(),
                    str_field(v, "sessionId").as_deref(),
                    &snap.commit_sha,
                );
                let _ = app.emit("snapshot://created", &snap);
            }
        }
    } else if kind == "turn_end" {
        // Testigo: the Stop hook closes the open turn — and the close carries
        // the turn's result: a post-turn snapshot diffed against the pre-turn
        // one, so the event answers "what did this turn change".
        let state = app.state::<AppState>();
        let project = state.inner.lock().project.clone();
        if let Some(p) = project {
            let root = p.root.to_string_lossy();
            let ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
            let term_id = str_field(v, "termId");

            let turn = term_id.as_deref().and_then(|t| state.testigo.peek_turn(t));
            // Where the turn ran: turn state from the prompt, else the stop
            // payload's cwd, else the project root (non-worktree default).
            let repo = turn
                .as_ref()
                .and_then(|t| t.cwd.clone())
                .or_else(|| str_field(v, "cwd"))
                .map(PathBuf::from)
                .filter(|c| c.is_dir())
                .unwrap_or_else(|| p.root.clone());
            let pre_sha = turn.as_ref().and_then(|t| t.pre_sha.clone());

            let mut payload = json!({});
            // The agent's closing words, when the CLI sends them (Claude
            // 2.1.47+ `last_assistant_message`). The hook already capped it;
            // this is the second guard, same as the tool-result excerpt.
            if let Some(summary) = str_field(v, "summary") {
                payload["summary"] = json!(truncate_chars(&summary, SUMMARY_MAX));
                payload["summaryTruncated"] = json!(
                    v.get("summaryTruncated")
                        .and_then(|t| t.as_bool())
                        .unwrap_or(false)
                        || summary.chars().count() > SUMMARY_MAX
                );
            }
            let post = snapshot_service::create(&repo, &uuid::Uuid::new_v4().to_string())
                .ok()
                .flatten();
            if let Some(post) = &post {
                payload["postSha"] = json!(post.commit_sha);
            }
            if let (Some(pre), Some(post)) = (&pre_sha, &post) {
                payload["preSha"] = json!(pre);
                if let Ok(files) = snapshot_service::diff_names(&repo, pre, &post.commit_sha) {
                    payload["filesTruncated"] =
                        json!(files.len() >= snapshot_service::DIFF_NAMES_MAX);
                    payload["filesChanged"] = json!(files
                        .iter()
                        .map(|(s, f)| json!({ "status": s, "path": f }))
                        .collect::<Vec<_>>());
                }
            }

            if let Ok(ev) = state.testigo.on_turn_end(
                root.as_ref(),
                ts,
                term_id.as_deref(),
                str_field(v, "sessionId").as_deref(),
                payload,
            ) {
                // V2-A: pin the new ledger head in the checkout's git refs —
                // best-effort, a non-git dir just skips. Repo marks are
                // opt-in per project (shared repos stay untouched by default).
                if state.testigo.repo_marks(root.as_ref()) {
                    let _ = crate::services::testigo_service::anchor_head(
                        &repo, ev.seq, &ev.hash, ev.ts,
                    );
                }
            }
        }
    } else if kind == "tool_result" {
        // Testigo: what one tool call produced inside the open turn.
        let state = app.state::<AppState>();
        let project = state.inner.lock().project.clone();
        if let Some(p) = project {
            let root = p.root.to_string_lossy();
            let ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
            let _ = state.testigo.on_tool_result(
                root.as_ref(),
                ts,
                str_field(v, "termId").as_deref(),
                str_field(v, "sessionId").as_deref(),
                str_field(v, "tool").as_deref(),
                str_field(v, "excerpt").as_deref(),
                v.get("truncated")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false),
            );
        }
    }
}

/// Read an optional string field from a hook payload, treating empty strings as
/// absent so the ledger doesn't store noise like `"skill": ""`.
fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// Cap on the turn summary stored in the ledger. Mirrors the hook script's own
/// cap; a hook script from an older install (or a hand-written event) doesn't
/// get to bloat the chain.
const SUMMARY_MAX: usize = 1000;

/// Truncate on a char boundary — the summary is prose from the agent, so it can
/// carry any UTF-8 and byte slicing would panic.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

fn approvals_watcher_loop(app: AppHandle, dir: PathBuf) {
    let (watcher, rx, interval) = dir_wake_source(&dir, Duration::from_millis(100));
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        await_wake(&rx, interval, watcher.is_some());
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut current: HashSet<String> = HashSet::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".req.json") {
                continue;
            }
            current.insert(name.clone());
            if seen.contains(&name) {
                continue;
            }
            let Ok(txt) = fs::read_to_string(e.path()) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&txt) else {
                continue;
            };
            let _ = app.emit("approval://request", &v);
            // Testigo: the request half of the approval audit trail. The raw
            // req/res files are polled and deleted by the hook; this line is
            // the durable record. Best-effort, never blocks the UI event.
            {
                let state = app.state::<AppState>();
                let project = state.inner.lock().project.clone();
                if let Some(p) = project {
                    let root = p.root.to_string_lossy();
                    let _ = state.testigo.on_approval_request(root.as_ref(), &v);
                }
            }
            seen.insert(name);
        }
        // Drop ids that disappeared (hook cleaned them up).
        seen.retain(|n| current.contains(n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// prune removes stale per-PID dirs, keeps the current one regardless of
    /// age, and ignores files. Cutoff is injected so the test doesn't need to
    /// fake mtimes: a future cutoff makes everything (except `keep`) stale.
    #[test]
    fn prune_removes_stale_session_dirs_keeps_current() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ac-prune-{}-{nanos}", std::process::id()));
        let keep = root.join("111");
        fs::create_dir_all(&keep).unwrap();
        fs::create_dir_all(root.join("222")).unwrap();
        fs::create_dir_all(root.join("333/approvals")).unwrap();
        fs::write(root.join("not-a-dir.txt"), b"x").unwrap();

        let future = SystemTime::now() + Duration::from_secs(60);
        let removed = prune_stale_session_dirs(&root, future, &keep);
        assert_eq!(removed, 2, "both stale dirs go, nested content included");
        assert!(keep.exists(), "current dir survives any cutoff");
        assert!(root.join("not-a-dir.txt").exists(), "plain files untouched");
        assert!(!root.join("222").exists() && !root.join("333").exists());

        // Recent dirs survive a normal (past) cutoff.
        fs::create_dir_all(root.join("444")).unwrap();
        let past = SystemTime::now() - Duration::from_secs(3600);
        assert_eq!(prune_stale_session_dirs(&root, past, &keep), 0);
        assert!(root.join("444").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn drain_consumes_complete_lines_incrementally() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ac-hooks-drain-{nanos}.jsonl"));

        // Missing file: nothing to do, offset unchanged.
        let (evs, off) = drain_new_lines(&path, 0);
        assert!(evs.is_empty());
        assert_eq!(off, 0);

        // Two complete events arrive.
        fs::write(&path, "{\"type\":\"a\"}\n{\"type\":\"b\"}\n").unwrap();
        let (evs, off) = drain_new_lines(&path, 0);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[1]["type"], "b");

        // No growth: nothing new.
        let (evs, off2) = drain_new_lines(&path, off);
        assert!(evs.is_empty());
        assert_eq!(off2, off);

        // A torn append (no trailing newline yet) must NOT be consumed…
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"type\":\"c\"").unwrap();
        f.flush().unwrap();
        let (evs, off3) = drain_new_lines(&path, off);
        assert!(evs.is_empty(), "half-written line stays buffered");
        assert_eq!(off3, off, "offset must not advance past torn tail");

        // …and is delivered intact once the writer finishes the line.
        f.write_all(b"}\n").unwrap();
        f.flush().unwrap();
        let (evs, off4) = drain_new_lines(&path, off3);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["type"], "c");
        assert!(off4 > off3);

        // Blank and malformed lines are skipped, valid neighbors still parse.
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"\nnot json\n{\"type\":\"d\"}\n").unwrap();
        drop(f);
        let (evs, off5) = drain_new_lines(&path, off4);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["type"], "d");

        // Truncation resets the offset so the next pass starts over.
        fs::write(&path, "{\"type\":\"fresh\"}\n").unwrap();
        let (evs, off6) = drain_new_lines(&path, off5);
        assert!(evs.is_empty(), "shrink pass only resets");
        assert_eq!(off6, 0);
        let (evs, _) = drain_new_lines(&path, off6);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["type"], "fresh");

        let _ = fs::remove_file(&path);
    }

    /// The hook command must survive cmd.exe: explicit `node` interpreter
    /// (no shebangs on Windows) and a quoted path (home dirs contain spaces —
    /// "C:\\Users\\Melissa Mujica\\…" is the real case that surfaced this).
    #[test]
    fn hook_command_is_quoted_and_node_prefixed() {
        let cmd = hook_command(Path::new("/home/user name/cache/userprompt-hook.cjs"));
        assert_eq!(cmd, "node \"/home/user name/cache/userprompt-hook.cjs\"");
    }

    #[test]
    fn upsert_installs_canonical_and_migrates_the_legacy_bare_path_format() {
        let script = Path::new("/x/hook with space.cjs");

        // Fresh install → canonical entry, once.
        let mut settings = json!({});
        upsert_hook(&mut settings, "UserPromptSubmit", script, None);
        upsert_hook(&mut settings, "UserPromptSubmit", script, None); // idempotent
        let arr = settings
            .pointer("/hooks/UserPromptSubmit")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr[0]
                .pointer("/hooks/0/command")
                .unwrap()
                .as_str()
                .unwrap(),
            "node \"/x/hook with space.cjs\""
        );

        // A legacy bare-path entry gets REPLACED, not duplicated; a foreign
        // hook in the same event is left alone.
        let mut settings = json!({
            "hooks": { "PreToolUse": [
                { "matcher": "*", "hooks": [{ "type": "command", "command": "/x/hook with space.cjs" }] },
                { "matcher": "*", "hooks": [{ "type": "command", "command": "some-other-tool" }] }
            ]}
        });
        upsert_hook(&mut settings, "PreToolUse", script, None);
        let arr = settings
            .pointer("/hooks/PreToolUse")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 2);
        let cmds: Vec<&str> = arr
            .iter()
            .map(|e| e.pointer("/hooks/0/command").unwrap().as_str().unwrap())
            .collect();
        assert!(cmds.contains(&"some-other-tool"));
        assert!(cmds.contains(&"node \"/x/hook with space.cjs\""));
        assert!(!cmds.contains(&"/x/hook with space.cjs"));
    }

    /// The install-order gap: bridge enabled first, codex CLI installed later.
    /// sync must mirror exactly the events installed claude-side (both command
    /// generations), leave foreign codex hooks alone, and be idempotent.
    #[test]
    fn sync_mirrors_claude_hooks_missing_on_the_codex_side() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("ac-sync-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let claude = base.join("claude-settings.json");
        let codex = base.join("codex").join("hooks.json");

        let prompt_script = PathBuf::from("/x/userprompt-hook.cjs");
        let pretool_script = PathBuf::from("/x/pretooluse-hook.cjs");
        let stop_script = PathBuf::from("/x/stop-hook.cjs");
        let pairs: [(&str, &PathBuf); 3] = [
            ("UserPromptSubmit", &prompt_script),
            ("PreToolUse", &pretool_script),
            ("Stop", &stop_script),
        ];

        // Claude side: observer in the LEGACY bare-path format plus the
        // opted-in bridge; Stop deliberately NOT installed. Codex side: a
        // foreign hook only (fresh codex install, trust prompt pending).
        let mut claude_settings = json!({
            "hooks": { "UserPromptSubmit": [
                { "matcher": "*", "hooks": [{ "type": "command", "command": "/x/userprompt-hook.cjs" }] }
            ]}
        });
        upsert_hook(&mut claude_settings, "PreToolUse", &pretool_script, None);
        write_settings_atomic(&claude, &claude_settings).unwrap();
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        write_settings_atomic(
            &codex,
            &json!({ "hooks": { "PreToolUse": [
                { "matcher": "*", "hooks": [{ "type": "command", "command": "some-other-tool" }] }
            ]}}),
        )
        .unwrap();

        assert_eq!(
            sync_hooks_between(&claude, &codex, &pairs, None).unwrap(),
            2
        );
        assert!(is_hook_installed(&codex, "UserPromptSubmit", &prompt_script).unwrap());
        assert!(is_hook_installed(&codex, "PreToolUse", &pretool_script).unwrap());
        assert!(
            !is_hook_installed(&codex, "Stop", &stop_script).unwrap(),
            "events not installed claude-side must not be mirrored"
        );
        let codex_root: Value = serde_json::from_str(&fs::read_to_string(&codex).unwrap()).unwrap();
        let pretool = codex_root
            .pointer("/hooks/PreToolUse")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(
            pretool
                .iter()
                .any(|e| entry_has_command(e, "some-other-tool")),
            "foreign codex hooks survive the sync"
        );

        // Idempotent: nothing missing ⇒ no write reported.
        assert_eq!(
            sync_hooks_between(&claude, &codex, &pairs, None).unwrap(),
            0
        );

        // A codex file that doesn't exist yet is created from scratch.
        let fresh = base.join("fresh").join("hooks.json");
        assert_eq!(
            sync_hooks_between(&claude, &fresh, &pairs, None).unwrap(),
            2
        );
        assert!(is_hook_installed(&fresh, "PreToolUse", &pretool_script).unwrap());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn has_command_path_matches_both_generations() {
        let script = Path::new("/x/h.cjs");
        let legacy = json!({ "hooks": [{ "type": "command", "command": "/x/h.cjs" }] });
        let canonical = json!({ "hooks": [{ "type": "command", "command": "node \"/x/h.cjs\"" }] });
        let other = json!({ "hooks": [{ "type": "command", "command": "node \"/y/other.cjs\"" }] });
        assert!(has_command_path(&legacy, script));
        assert!(has_command_path(&canonical, script));
        assert!(!has_command_path(&other, script));
    }

    #[test]
    fn upsert_with_binary_writes_the_bridge_command_and_migrates_the_node_form() {
        let script = Path::new("/x/userprompt-hook.cjs");
        let binary = Path::new("/cache/bin/hook-bridge");
        // Existing install in the node form.
        let mut settings = json!({});
        upsert_hook(&mut settings, "UserPromptSubmit", script, None);
        let arr = settings
            .pointer("/hooks/UserPromptSubmit")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert!(has_command_path(&arr[0], script));
        // Re-install with the binary available: migrated, not duplicated.
        upsert_hook(&mut settings, "UserPromptSubmit", script, Some(binary));
        let arr = settings
            .pointer("/hooks/UserPromptSubmit")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 1, "node-form entry migrated, not duplicated");
        assert!(has_bridge_command(&arr[0], "userprompt"));
        // Idempotent on the bridge form too.
        upsert_hook(&mut settings, "UserPromptSubmit", script, Some(binary));
        assert_eq!(
            settings
                .pointer("/hooks/UserPromptSubmit")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        // And a binary that MOVED (old cache path) is migrated as well.
        let moved = Path::new("/elsewhere/bin/hook-bridge");
        upsert_hook(&mut settings, "UserPromptSubmit", script, Some(moved));
        let arr = settings
            .pointer("/hooks/UserPromptSubmit")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 1);
        let cmd = arr[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.starts_with("\"/elsewhere/"));
    }

    #[test]
    fn bridge_entries_read_as_installed_and_uninstall_cleanly() {
        let entry = json!({ "hooks": [{ "type": "command",
            "command": "\"/cache/bin/hook-bridge\" pretooluse" }] });
        assert!(has_bridge_command(&entry, "pretooluse"));
        assert!(!has_bridge_command(&entry, "stop"));
        // Mode must be the trailing word — a path merely containing the name
        // does not match another mode's entry.
        let other = json!({ "hooks": [{ "type": "command",
            "command": "node \"/x/hook-bridge-notes.cjs\"" }] });
        assert!(!has_bridge_command(&other, "pretooluse"));
    }

    /// PostModelSwitch is Claude-only, so it must get its OWN bridge mode —
    /// sharing `stop` would make install/uninstall unable to tell the two
    /// entries apart (the mode word is how we recognize our own entries).
    #[test]
    fn model_switch_has_its_own_bridge_mode_and_installs_only_for_claude() {
        assert_eq!(bridge_mode("PostModelSwitch"), "modelswitch");
        assert_ne!(bridge_mode("PostModelSwitch"), bridge_mode("Stop"));

        let script = Path::new("/cache/modelswitch-hook.cjs");
        let bin = Path::new("/cache/bin/hook-bridge");
        let mut settings = json!({});
        upsert_hook(&mut settings, "PostModelSwitch", script, Some(bin));
        upsert_hook(
            &mut settings,
            "Stop",
            Path::new("/cache/stop-hook.cjs"),
            Some(bin),
        );
        let arr = settings
            .pointer("/hooks/PostModelSwitch")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert!(has_bridge_command(&arr[0], "modelswitch"));
        assert!(!has_bridge_command(&arr[0], "stop"));
        // …and the Stop entry is untouched by it.
        let stop = settings.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert!(has_bridge_command(&stop[0], "stop"));
    }

    /// Run the REAL modelswitch script under node: it must record the model the
    /// session moved to, ignore a subagent's own switch, and stay silent when
    /// the payload has no destination.
    #[cfg(unix)]
    #[test]
    fn modelswitch_bridge_records_session_switches_and_ignores_subagents() {
        use std::io::Write as _;
        use std::process::{Command, Stdio};

        if Command::new("node")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("node not available — skipping modelswitch bridge test");
            return;
        }

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let session_dir =
            std::env::temp_dir().join(format!("ac-modelswitch-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&session_dir).unwrap();

        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/modelswitch-hook.cjs");
        let run = |payload: String| {
            let mut child = Command::new("node")
                .arg(&script)
                .env("AGENT_CONSOLE_SESSION_DIR", &session_dir)
                .env("AGENT_CONSOLE_TERM_ID", "term-7")
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.as_bytes())
                .unwrap();
            assert!(child.wait().unwrap().success(), "bridge must always exit 0");
        };

        run(
            r#"{"session_id":"s1","from_model":"claude-sonnet-5","to_model":"claude-opus-5"}"#
                .into(),
        );
        // A subagent's switch says nothing about the session's own model.
        run(r#"{"session_id":"s1","to_model":"claude-haiku-4-5","agent_id":"sub-1"}"#.into());
        // Nothing to report.
        run(r#"{"session_id":"s1","from_model":"claude-opus-5"}"#.into());

        let events = fs::read_to_string(session_dir.join("events.jsonl")).unwrap();
        let lines: Vec<Value> = events
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 1, "only the session's own switch is recorded");
        assert_eq!(lines[0]["type"], "model_switch");
        assert_eq!(lines[0]["model"], "claude-opus-5");
        assert_eq!(lines[0]["fromModel"], "claude-sonnet-5");
        assert_eq!(lines[0]["sessionId"], "s1");
        assert_eq!(lines[0]["termId"], "term-7");

        let _ = fs::remove_dir_all(&session_dir);
    }

    #[test]
    fn summary_truncation_respects_char_boundaries() {
        assert_eq!(truncate_chars("short", SUMMARY_MAX), "short");
        // Multi-byte prose: cutting by bytes here would panic or corrupt.
        let long: String = "áé—".repeat(SUMMARY_MAX);
        let cut = truncate_chars(&long, SUMMARY_MAX);
        assert_eq!(cut.chars().count(), SUMMARY_MAX);
        assert!(long.starts_with(&cut));
    }

    /// The Stop bridge is what closes a turn in the ledger. Run the real script
    /// under node and assert the event it appends: the agent's closing words
    /// when the CLI sends them (Claude 2.1.47+ `last_assistant_message`),
    /// bounded, and nothing extra when it doesn't (Codex today).
    #[cfg(unix)]
    #[test]
    fn stop_bridge_records_the_agents_closing_words_when_the_cli_sends_them() {
        use std::io::Write as _;
        use std::process::{Command, Stdio};

        if Command::new("node")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("node not available — skipping stop bridge test");
            return;
        }

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let session_dir =
            std::env::temp_dir().join(format!("ac-stop-test-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&session_dir).unwrap();

        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/stop-hook.cjs");
        let run = |payload: String| {
            let mut child = Command::new("node")
                .arg(&script)
                .env("AGENT_CONSOLE_SESSION_DIR", &session_dir)
                .env("AGENT_CONSOLE_TERM_ID", "term-7")
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.as_bytes())
                .unwrap();
            assert!(child.wait().unwrap().success(), "bridge must always exit 0");
        };

        run(r#"{"session_id":"s1","cwd":"/proj/x","last_assistant_message":"  Fixed the retry loop and added a test.  "}"#.into());
        // Chatty turn: capped, and the event says it was cut.
        let long = "x".repeat(2500);
        run(format!(
            r#"{{"session_id":"s1","last_assistant_message":"{long}"}}"#
        ));
        // No such field (Codex, older Claude): the event is what it always was.
        run(r#"{"session_id":"s1","cwd":"/proj/x"}"#.into());

        let events = fs::read_to_string(session_dir.join("events.jsonl")).unwrap();
        let lines: Vec<Value> = events
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);

        assert_eq!(lines[0]["type"], "turn_end");
        assert_eq!(lines[0]["termId"], "term-7");
        assert_eq!(
            lines[0]["summary"], "Fixed the retry loop and added a test.",
            "the closing words ride the turn close, trimmed"
        );
        assert_eq!(lines[0]["summaryTruncated"], false);

        assert_eq!(lines[1]["summary"].as_str().unwrap().len(), 1000);
        assert_eq!(lines[1]["summaryTruncated"], true);

        assert!(
            lines[2].get("summary").is_none(),
            "a CLI that doesn't report it must not produce an empty summary"
        );

        let _ = fs::remove_dir_all(&session_dir);
    }

    /// End-to-end over the REAL userprompt bridge: run it under node against a
    /// fake loopback inject endpoint and assert both of its jobs — the events
    /// log append and the additionalContext output. Skips silently when node
    /// isn't installed (the script only ever runs where node exists: hook
    /// commands are `node "<path>"`).
    #[cfg(unix)]
    #[test]
    fn userprompt_bridge_injects_context_and_degrades_without_port_file() {
        use std::io::{Read as _, Write as _};
        use std::process::{Command, Stdio};

        if Command::new("node")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("node not available — skipping bridge test");
            return;
        }

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-bridge-test-{}-{nanos}", std::process::id()));
        let session_dir = base.join("session");
        let data_dir = base.join("data").join("agent-console");
        fs::create_dir_all(&session_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();

        // Fake inject endpoint: two connections, one answer each — first the
        // full answer (context + title), then a title-only one (nothing to
        // inject, but the session still has a name to export).
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let bodies = [
                r#"{"context":"CTX-FROM-APP","sessionTitle":"Fix login on windows"}"#,
                r#"{"context":null,"sessionTitle":"Fix login on windows"}"#,
            ];
            for body in bodies {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        fs::write(
            data_dir.join("inject-port.json"),
            format!("{{\"port\":{port},\"pid\":1}}"),
        )
        .unwrap();

        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/userprompt-hook.cjs");
        let run = |xdg: &Path| -> String {
            let mut child = Command::new("node")
                .arg(&script)
                .env("AGENT_CONSOLE_SESSION_DIR", &session_dir)
                .env("XDG_DATA_HOME", xdg)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(br#"{"prompt":"how do we cut a release here?","cwd":"/proj/x","session_id":"s1"}"#)
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "bridge must always exit 0");
            String::from_utf8_lossy(&out.stdout).to_string()
        };

        // With the port file: context comes back as hookSpecificOutput.
        let stdout = run(&base.join("data"));
        let v: Value = serde_json::from_str(stdout.trim()).expect("bridge output is JSON");
        assert_eq!(
            v.pointer("/hookSpecificOutput/additionalContext")
                .and_then(Value::as_str),
            Some("CTX-FROM-APP")
        );
        assert_eq!(
            v.pointer("/hookSpecificOutput/hookEventName")
                .and_then(Value::as_str),
            Some("UserPromptSubmit")
        );
        // The session name the app exports rides the same answer, so the CLI's
        // own `/resume` list shows what our sidebar shows (Claude 2.1.94).
        assert_eq!(
            v.pointer("/hookSpecificOutput/sessionTitle")
                .and_then(Value::as_str),
            Some("Fix login on windows")
        );

        // Title without context: still an output, and no empty context field.
        let stdout = run(&base.join("data"));
        let v: Value = serde_json::from_str(stdout.trim()).expect("bridge output is JSON");
        assert_eq!(
            v.pointer("/hookSpecificOutput/sessionTitle")
                .and_then(Value::as_str),
            Some("Fix login on windows")
        );
        assert!(
            v.pointer("/hookSpecificOutput/additionalContext").is_none(),
            "nothing to inject → the field must be absent, not null"
        );
        server.join().unwrap();

        // Without a port file (app closed): silent, but the event still logs.
        let stdout = run(&base.join("nowhere"));
        assert!(stdout.trim().is_empty(), "no app → no output, no error");
        let events = fs::read_to_string(session_dir.join("events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 3, "every run must log the prompt");
        assert!(events.contains("how do we cut a release"));
    }
}
