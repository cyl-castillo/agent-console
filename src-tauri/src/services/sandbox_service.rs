//! SEGURIDAD-MOVIL F2: is the engine's sandbox on for this project?
//!
//! The one control that makes a forged approval impossible rather than
//! merely detected: Claude Code's sandbox (`sandbox.enabled` in
//! settings.json) runs the Bash tool with writes confined to the project
//! and no loopback network, so the agent's own shell cannot touch the
//! console's answer files or its loopback endpoint. Hooks run outside it,
//! so the console's bridge keeps working. Codex has the same property in
//! `workspace-write`; OpenCode has no sandbox yet.
//!
//! This reads the settings files the CLI reads (project local, project,
//! user — in that precedence) and can switch the flag on in the project's
//! settings, merging into whatever else is there.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatus {
    /// `Some(true)` on, `Some(false)` explicitly off, `None` never set.
    pub enabled: Option<bool>,
    /// Which file decided it: `project-local`, `project`, `user`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Where "Enable" would write.
    pub project_settings: String,
}

fn read_json(path: &Path) -> Option<Value> {
    let txt = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&txt).ok()
}

fn flag(v: &Value) -> Option<bool> {
    v.get("sandbox")?.get("enabled")?.as_bool()
}

/// The three files the CLI consults, most specific first.
fn candidates(project_root: &Path, home: Option<&Path>) -> Vec<(&'static str, PathBuf)> {
    let mut out = vec![
        (
            "project-local",
            project_root.join(".claude").join("settings.local.json"),
        ),
        (
            "project",
            project_root.join(".claude").join("settings.json"),
        ),
    ];
    if let Some(h) = home {
        out.push(("user", h.join(".claude").join("settings.json")));
    }
    out
}

pub fn status_in(project_root: &Path, home: Option<&Path>) -> SandboxStatus {
    for (source, path) in candidates(project_root, home) {
        if let Some(enabled) = read_json(&path).as_ref().and_then(flag) {
            return SandboxStatus {
                enabled: Some(enabled),
                source: Some(source.into()),
                project_settings: project_root
                    .join(".claude")
                    .join("settings.json")
                    .to_string_lossy()
                    .to_string(),
            };
        }
    }
    SandboxStatus {
        enabled: None,
        source: None,
        project_settings: project_root
            .join(".claude")
            .join("settings.json")
            .to_string_lossy()
            .to_string(),
    }
}

pub fn status(project_root: &Path) -> SandboxStatus {
    status_in(project_root, dirs::home_dir().as_deref())
}

/// Switch the sandbox on in the project's `.claude/settings.json`, keeping
/// everything else in the file. A project-local `false` would still win;
/// the status says so afterwards.
pub fn enable(project_root: &Path) -> AppResult<SandboxStatus> {
    let dir = project_root.join(".claude");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");
    let mut settings = read_json(&path).unwrap_or_else(|| json!({}));
    if !settings.is_object() {
        return Err(AppError::Other(format!(
            "{} is not a JSON object",
            path.display()
        )));
    }
    let sandbox = settings
        .as_object_mut()
        .expect("object")
        .entry("sandbox")
        .or_insert_with(|| json!({}));
    if !sandbox.is_object() {
        *sandbox = json!({});
    }
    sandbox["enabled"] = json!(true);
    crate::services::hooks_service::write_settings_atomic(&path, &settings)?;
    Ok(status(project_root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("ac-sandbox-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_the_flag_in_the_clis_precedence_and_enables_it_without_losing_the_rest() {
        let root = temp();
        let home = temp();
        // Nothing set anywhere.
        let st = status_in(&root, Some(&home));
        assert_eq!(st.enabled, None);
        // User says on, project says nothing → on, from the user file.
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"sandbox":{"enabled":true}}"#,
        )
        .unwrap();
        let st = status_in(&root, Some(&home));
        assert_eq!(
            (st.enabled, st.source.as_deref()),
            (Some(true), Some("user"))
        );
        // Project-local off wins over everything.
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude/settings.local.json"),
            r#"{"sandbox":{"enabled":false}}"#,
        )
        .unwrap();
        let st = status_in(&root, Some(&home));
        assert_eq!(
            (st.enabled, st.source.as_deref()),
            (Some(false), Some("project-local"))
        );
        // Enable writes the project file and keeps its other keys.
        std::fs::write(
            root.join(".claude/settings.json"),
            r#"{"hooks":{"Stop":[]},"sandbox":{"network":{"allowedDomains":["github.com"]}}}"#,
        )
        .unwrap();
        enable(&root).unwrap();
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["sandbox"]["enabled"], true);
        assert_eq!(
            written["sandbox"]["network"]["allowedDomains"][0],
            "github.com"
        );
        assert!(written["hooks"]["Stop"].is_array());
        // A non-object file is refused, not clobbered.
        std::fs::write(root.join(".claude/settings.json"), "[1,2]").unwrap();
        assert!(enable(&root).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join(".claude/settings.json")).unwrap(),
            "[1,2]"
        );
    }
}
