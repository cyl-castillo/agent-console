//! OpenCode specifics the engine adapter cannot express as argv: the MCP
//! entry it reads from `opencode.json` in the working folder (OpenCode takes
//! no `--mcp-config`), and its live model catalog (`provider/model` ids from
//! every provider the user configured — there is no stable alias list to
//! hard-code, unlike Claude's `opus`/`sonnet`).
//!
//! Port of ai-connector's `integrations/opencode.py` and
//! `opencode_models.py` (Marcos Macías, with permission; he targets OpenCode
//! V2), adjusted to what the installed CLI accepts: OpenCode V1 (1.19.x, the
//! `curl | bash` install of 2026-10) rejects the V2 `mcp.servers` /
//! `permissions` shapes ("V2 permissions are not supported by OpenCode V1")
//! and has no `opencode api`. The V1 shapes below — `mcp.<name>` and the
//! `permission` object — are still accepted by V2 (deprecated, not removed),
//! so one writer serves both. Verified empirically with `opencode mcp list`
//! and `opencode debug config` on 1.19.3.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::services::claude_cli;

/// The server name OpenCode registers the connector under. OpenCode names
/// MCP tools `<server>_<tool>`, so the agent sees `agent_console_delegate_task`.
pub const SERVER_NAME: &str = "agent_console";
/// `opencode reload` is best effort: a running OpenCode service for the folder
/// rereads its config on it; nothing waits long for that.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(10);
const MODELS_TIMEOUT: Duration = Duration::from_secs(30);

/// Write (or refresh) the connector's entry in `<cwd>/opencode.json`, keeping
/// every other key the user has there. Returns whether the file changed.
/// Refuses to replace an `agent_console` entry that is not ours. Only the
/// MCP entry is written: OpenCode's built-in `plan` agent is already
/// read-only, and the connector's tools need no permission rule in V1.
pub fn ensure_config_entry(cwd: &Path, bridge: &Path, project: &str, job: &str) -> AppResult<bool> {
    let path = cwd.join("opencode.json");
    // A JSONC variant wins in OpenCode's lookup; editing the JSON would be ignored.
    if cwd.join("opencode.jsonc").exists() || cwd.join(".opencode").join("opencode.jsonc").exists()
    {
        return Err(AppError::InvalidArgument(
            "the folder has an opencode.jsonc; add the agent_console MCP entry there by hand"
                .into(),
        ));
    }
    let current = if path.exists() {
        Some(fs::read_to_string(&path)?)
    } else {
        None
    };
    let mut config: Value = match &current {
        Some(txt) if !txt.trim().is_empty() => serde_json::from_str(txt).map_err(|e| {
            AppError::InvalidArgument(format!("opencode.json is not valid JSON: {e}"))
        })?,
        _ => json!({}),
    };
    if !config.is_object() {
        return Err(AppError::InvalidArgument(
            "opencode.json is not a JSON object".into(),
        ));
    }
    let spec = entry_spec(bridge, project, job);
    let mcp = config
        .as_object_mut()
        .unwrap()
        .entry("mcp")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InvalidArgument("opencode.json: `mcp` is not an object".into()))?;
    // A V2 file keeps its servers under `mcp.servers`; put ours beside theirs
    // there. A V1 file (or a fresh one) takes the top-level map.
    let servers = if matches!(mcp.get("servers"), Some(Value::Object(_))) {
        mcp.get_mut("servers").unwrap().as_object_mut().unwrap()
    } else {
        mcp
    };
    if let Some(existing) = servers.get(SERVER_NAME) {
        if !is_own_entry(existing) {
            return Err(AppError::InvalidArgument(
                "another agent_console MCP entry already exists in opencode.json; it will not be overwritten".into(),
            ));
        }
    }
    servers.insert(SERVER_NAME.into(), spec);
    let updated = format!(
        "{}\n",
        serde_json::to_string_pretty(&config).unwrap_or_default()
    );
    if current.as_deref() == Some(updated.as_str()) {
        return Ok(false);
    }
    write_atomically(&path, &updated)?;
    reload(cwd);
    Ok(true)
}

fn entry_spec(bridge: &Path, project: &str, job: &str) -> Value {
    // No `--caller`: one file serves every participant of the folder, so the
    // app identifies the caller by OpenCode's session id or by the room's
    // turn in flight. `enabled` is the V1 key; V2 reads it too.
    json!({
        "type": "local",
        "command": [
            bridge.to_string_lossy(),
            "mcp",
            "--project", project,
            "--job", job,
        ],
        "enabled": true,
    })
}

fn is_own_entry(entry: &Value) -> bool {
    entry
        .pointer("/command")
        .and_then(Value::as_array)
        .and_then(|c| c.get(1))
        .and_then(Value::as_str)
        == Some("mcp")
        && entry
            .pointer("/command/0")
            .and_then(Value::as_str)
            .is_some_and(|c| c.contains("hook-bridge"))
}

fn write_atomically(path: &Path, content: &str) -> AppResult<()> {
    // A symlinked config is updated at its target, keeping the link.
    let target: PathBuf = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let tmp = target.with_file_name(format!(
        ".opencode.json.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, &target)?;
    Ok(())
}

fn reload(cwd: &Path) {
    let mut cmd = claude_cli::opencode_command_in(cwd, &["reload"]);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    if let Ok(mut child) = cmd.spawn() {
        let deadline = std::time::Instant::now() + RELOAD_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                _ => {
                    let _ = child.kill();
                    break;
                }
            }
        }
    }
}

/// One model OpenCode offers in a folder, as the picker shows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpencodeModel {
    /// `provider/model` — what `opencode run --model` takes.
    pub id: String,
    pub name: String,
    pub provider: String,
    /// Reasoning variants the model exposes (`--variant`). V1's `models`
    /// listing names none; V2's catalog does.
    #[serde(default)]
    pub variants: Vec<String>,
}

/// The models of every provider configured for `dir`, from OpenCode itself:
/// `opencode models` prints one `provider/model` per line (V1 and V2 alike;
/// V2's richer `api GET /api/model` is not in V1).
pub fn list_models(dir: &Path) -> AppResult<Vec<OpencodeModel>> {
    let mut cmd = claude_cli::opencode_command_in(dir, &["models"]);
    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Other(format!("failed to spawn `opencode`: {e}. Is it on PATH?")))?;
    let deadline = std::time::Instant::now() + MODELS_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                return Err(AppError::Other(
                    "OpenCode took too long to list its models. Try again.".into(),
                ));
            }
        }
    };
    let out = child
        .wait_with_output()
        .map_err(|e| AppError::Other(format!("opencode models: {e}")))?;
    if !status.success() {
        return Err(AppError::Other(
            "OpenCode could not list its models. Check its providers in this folder.".into(),
        ));
    }
    Ok(parse_models(&String::from_utf8_lossy(&out.stdout)))
}

/// One `provider/model` per line; banner lines and blanks are skipped.
pub fn parse_models(raw: &str) -> Vec<OpencodeModel> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('[') && l.contains('/'))
        .filter(|l| {
            l.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':'))
        })
        .filter_map(|l| {
            let (provider, model) = l.split_once('/')?;
            Some(OpencodeModel {
                id: l.to_string(),
                name: model.to_string(),
                provider: provider.to_string(),
                variants: Vec::new(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ac-opencode-{tag}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_entry(dir: &Path) -> bool {
        // `reload` spawns `opencode`; absent, the spawn fails silently.
        ensure_config_entry(dir, Path::new("/cache/bin/hook-bridge"), "/proj", "room-1").unwrap()
    }

    fn read_cfg(dir: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(dir.join("opencode.json")).unwrap()).unwrap()
    }

    #[test]
    fn a_v2_config_gets_the_entry_beside_the_users_servers() {
        let dir = tmp("v2");
        fs::write(
            dir.join("opencode.json"),
            r#"{"model":"anthropic/claude-sonnet-4","mcp":{"servers":{"mine":{"type":"remote","url":"https://x"}}},"permissions":[{"action":"edit","resource":"*","effect":"ask"}]}"#,
        )
        .unwrap();
        assert!(write_entry(&dir), "first write changes the file");
        let cfg = read_cfg(&dir);
        assert_eq!(cfg["model"], "anthropic/claude-sonnet-4");
        assert_eq!(cfg["mcp"]["servers"]["mine"]["url"], "https://x");
        let ours = &cfg["mcp"]["servers"]["agent_console"];
        assert_eq!(ours["type"], "local");
        assert_eq!(
            ours["command"],
            json!([
                "/cache/bin/hook-bridge",
                "mcp",
                "--project",
                "/proj",
                "--job",
                "room-1"
            ])
        );
        assert_eq!(ours["enabled"], true);
        // The user's permission rules are not touched.
        assert_eq!(cfg["permissions"].as_array().unwrap().len(), 1);
        assert!(cfg.get("agents").is_none());
        // Idempotent: the same entry again changes nothing.
        assert!(!write_entry(&dir));
        assert!(!dir.join("opencode.json.tmp").exists());
        // A foreign agent_console entry is never overwritten.
        let mut cfg2 = cfg.clone();
        cfg2["mcp"]["servers"]["agent_console"] =
            json!({ "type": "remote", "url": "https://theirs" });
        fs::write(dir.join("opencode.json"), cfg2.to_string()).unwrap();
        assert!(ensure_config_entry(&dir, Path::new("/b/hook-bridge"), "/proj", "room-1").is_err());
        // A jsonc sibling wins in OpenCode: refuse instead of writing a file it ignores.
        let dir2 = tmp("jsonc");
        fs::write(dir2.join("opencode.jsonc"), "{}").unwrap();
        assert!(ensure_config_entry(&dir2, Path::new("/b/hook-bridge"), "/proj", "r").is_err());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn a_missing_or_v1_config_takes_the_top_level_mcp_map() {
        // Fresh file: the V1 shape, which V1 (installed) and V2 both read.
        let dir = tmp("fresh");
        assert!(write_entry(&dir));
        let cfg = read_cfg(&dir);
        assert_eq!(cfg["mcp"]["agent_console"]["type"], "local");
        assert_eq!(cfg["mcp"]["agent_console"]["enabled"], true);
        assert!(cfg["mcp"].get("servers").is_none());
        // An existing V1 file with the user's own server: ours beside it.
        let dir2 = tmp("v1");
        fs::write(
            dir2.join("opencode.json"),
            r#"{"mcp":{"theirs":{"type":"local","command":["x"],"enabled":true}},"permission":{"edit":"ask"}}"#,
        )
        .unwrap();
        assert!(write_entry(&dir2));
        let cfg = read_cfg(&dir2);
        assert_eq!(cfg["mcp"]["theirs"]["command"], json!(["x"]));
        assert!(cfg["mcp"]["agent_console"].is_object());
        assert_eq!(cfg["permission"]["edit"], "ask");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn model_lines_become_provider_slash_model_ids() {
        // Real output of `opencode models` on 1.19.3 (banner-free).
        let raw = "opencode/beam-free\nopencode/big-pickle\n\nanthropic/claude-sonnet-4\n[INFO] noise\nnot a model\n";
        let rows = parse_models(raw);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "opencode/beam-free");
        assert_eq!(rows[0].provider, "opencode");
        assert_eq!(rows[0].name, "beam-free");
        assert_eq!(rows[2].id, "anthropic/claude-sonnet-4");
        assert!(parse_models("").is_empty());
    }
}
