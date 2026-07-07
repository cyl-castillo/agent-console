use std::fs;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Plugins are managed entirely through the `claude` CLI so we stay in lockstep
/// with what Claude Code can actually install/enable. No network scraping, no
/// hardcoded marketplace URL — the source of truth is the user's configured
/// marketplaces (cloned under ~/.claude/plugins/marketplaces).

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    /// Full id, e.g. "rust-analyzer-lsp@claude-plugins-official".
    pub id: String,
    pub name: String,
    pub marketplace: Option<String>,
    pub version: Option<String>,
    pub scope: Option<String>,
    pub enabled: bool,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePlugin {
    /// Id to pass to `claude plugin install`, e.g. "name@marketplace".
    pub install_id: String,
    pub name: String,
    pub marketplace: String,
    pub description: String,
    pub author: Option<String>,
    pub category: Option<String>,
    pub homepage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableSnapshot {
    /// Names of configured marketplaces (empty => user must add one).
    pub marketplaces: Vec<String>,
    pub plugins: Vec<MarketplacePlugin>,
}

/// Build a `claude` command via the shared resolver (so a GUI launch without a
/// login-shell PATH still finds the binary). stdio + Windows no-window set there.
fn claude_command(args: &[&str]) -> Command {
    crate::services::claude_cli::command(args)
}

/// Run `claude <args>` and return stdout on success, or a useful error.
fn run_claude(args: &[&str]) -> AppResult<String> {
    let output = claude_command(args).output().map_err(|e| {
        AppError::Other(format!(
            "failed to run `claude {}`: {e}. Is it on PATH?",
            args.join(" ")
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr
        } else {
            stdout
        };
        return Err(AppError::Other(format!(
            "claude {} failed: {}",
            args.join(" "),
            detail.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliInstalled {
    id: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    install_path: Option<String>,
}

/// Installed plugins, from `claude plugin list --json`.
pub fn list_installed() -> Vec<InstalledPlugin> {
    let Ok(stdout) = run_claude(&["plugin", "list", "--json"]) else {
        return Vec::new();
    };
    let parsed: Vec<CliInstalled> = serde_json::from_str(stdout.trim()).unwrap_or_default();
    parsed
        .into_iter()
        .map(|p| {
            let (name, marketplace) = split_id(&p.id);
            InstalledPlugin {
                id: p.id,
                name,
                marketplace,
                version: p.version,
                scope: p.scope,
                enabled: p.enabled.unwrap_or(true),
                path: p.install_path,
            }
        })
        .collect()
}

fn split_id(id: &str) -> (String, Option<String>) {
    match id.split_once('@') {
        Some((name, mk)) => (name.to_string(), Some(mk.to_string())),
        None => (id.to_string(), None),
    }
}

#[derive(Deserialize)]
struct CliMarketplace {
    name: String,
    #[serde(default)]
    #[serde(rename = "installLocation")]
    install_location: Option<String>,
}

// Shape of <marketplace>/.claude-plugin/marketplace.json
#[derive(Deserialize)]
struct MarketplaceManifest {
    #[serde(default)]
    plugins: Vec<ManifestPlugin>,
}

#[derive(Deserialize)]
struct ManifestPlugin {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: Option<AuthorField>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AuthorField {
    Name { name: String },
    Plain(String),
}

impl AuthorField {
    fn into_string(self) -> String {
        match self {
            AuthorField::Name { name } => name,
            AuthorField::Plain(s) => s,
        }
    }
}

/// Available plugins across all configured marketplaces. Reads the locally
/// cloned marketplace manifests so the catalogue matches exactly what
/// `claude plugin install` can resolve.
pub fn list_available() -> AvailableSnapshot {
    let mut marketplaces: Vec<String> = Vec::new();
    let mut plugins: Vec<MarketplacePlugin> = Vec::new();

    let cli = match run_claude(&["plugin", "marketplace", "list", "--json"]) {
        Ok(s) => s,
        Err(_) => {
            return AvailableSnapshot {
                marketplaces,
                plugins,
            }
        }
    };
    let configured: Vec<CliMarketplace> = serde_json::from_str(cli.trim()).unwrap_or_default();

    for mk in configured {
        marketplaces.push(mk.name.clone());
        let Some(loc) = mk.install_location else {
            continue;
        };
        let manifest_path = Path::new(&loc)
            .join(".claude-plugin")
            .join("marketplace.json");
        let Ok(text) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<MarketplaceManifest>(&text) else {
            continue;
        };
        for p in manifest.plugins {
            plugins.push(MarketplacePlugin {
                install_id: format!("{}@{}", p.name, mk.name),
                name: p.name,
                marketplace: mk.name.clone(),
                description: p.description,
                author: p.author.map(AuthorField::into_string),
                category: p.category,
                homepage: p.homepage,
            });
        }
    }

    plugins.sort_by_key(|a| a.name.to_lowercase());
    AvailableSnapshot {
        marketplaces,
        plugins,
    }
}

/// Ids come from our own catalogue / the CLI's own list, but validate
/// defensively before anything reaches a subprocess argument.
fn validate_plugin_id(id: &str) -> AppResult<()> {
    if id.trim().is_empty() {
        return Err(AppError::InvalidArgument("empty plugin id".into()));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | '/'))
    {
        return Err(AppError::InvalidArgument(format!("invalid plugin id: {id}")));
    }
    // A leading '-' would be parsed by the CLI as a flag, not an id.
    if id.starts_with('-') {
        return Err(AppError::InvalidArgument(format!("invalid plugin id: {id}")));
    }
    Ok(())
}

fn normalize_scope(scope: &str) -> &str {
    match scope {
        "user" | "project" | "local" => scope,
        _ => "user",
    }
}

/// Install a plugin via `claude plugin install <id> --scope <scope>`.
/// Returns the CLI's stdout (a short success line) on success.
pub fn install_plugin(install_id: &str, scope: &str) -> AppResult<String> {
    validate_plugin_id(install_id)?;
    let scope = normalize_scope(scope);
    let out = run_claude(&["plugin", "install", install_id, "--scope", scope])?;
    Ok(out.trim().to_string())
}

/// Update an installed plugin via `claude plugin update <id> --scope <scope>`.
/// The CLI notes a restart is required to apply — callers surface that.
pub fn update_plugin(id: &str, scope: &str) -> AppResult<String> {
    validate_plugin_id(id)?;
    let scope = normalize_scope(scope);
    let out = run_claude(&["plugin", "update", id, "--scope", scope])?;
    Ok(out.trim().to_string())
}

/// Refresh every configured marketplace from its source so the catalogue (and
/// the versions `plugin update` resolves against) is current.
pub fn update_marketplaces() -> AppResult<String> {
    let out = run_claude(&["plugin", "marketplace", "update"])?;
    Ok(out.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_validation_accepts_real_ids_and_rejects_injection() {
        assert!(validate_plugin_id("code-review@anthropic-skills").is_ok());
        assert!(validate_plugin_id("my_plugin.v2").is_ok());
        assert!(validate_plugin_id("org/repo@mk").is_ok());
        assert!(validate_plugin_id("").is_err());
        assert!(validate_plugin_id("   ").is_err());
        assert!(validate_plugin_id("evil; rm -rf /").is_err());
        // Valid charset but would be parsed by the CLI as a flag, not an id.
        assert!(validate_plugin_id("--scope").is_err());
    }

    #[test]
    fn unknown_scopes_fall_back_to_user() {
        assert_eq!(normalize_scope("project"), "project");
        assert_eq!(normalize_scope("local"), "local");
        assert_eq!(normalize_scope("managed"), "user");
        assert_eq!(normalize_scope("anything"), "user");
    }

    #[test]
    fn split_id_separates_marketplace() {
        assert_eq!(
            split_id("name@mk"),
            ("name".to_string(), Some("mk".to_string()))
        );
        assert_eq!(split_id("solo"), ("solo".to_string(), None));
    }
}
