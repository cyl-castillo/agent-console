use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

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

/// Build a `claude` command with stdio piped and, on Windows, no console window.
fn claude_command(args: &[&str]) -> Command {
    let mut cmd = Command::new("claude");
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

/// Run `claude <args>` and return stdout on success, or a useful error.
fn run_claude(args: &[&str]) -> AppResult<String> {
    let output = claude_command(args).output().map_err(|e| {
        AppError::Other(format!("failed to run `claude {}`: {e}. Is it on PATH?", args.join(" ")))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() { stderr } else { stdout };
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
        Err(_) => return AvailableSnapshot { marketplaces, plugins },
    };
    let configured: Vec<CliMarketplace> = serde_json::from_str(cli.trim()).unwrap_or_default();

    for mk in configured {
        marketplaces.push(mk.name.clone());
        let Some(loc) = mk.install_location else { continue; };
        let manifest_path = Path::new(&loc).join(".claude-plugin").join("marketplace.json");
        let Ok(text) = fs::read_to_string(&manifest_path) else { continue; };
        let Ok(manifest) = serde_json::from_str::<MarketplaceManifest>(&text) else { continue; };
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

    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    AvailableSnapshot { marketplaces, plugins }
}

/// Install a plugin via `claude plugin install <id> --scope <scope>`.
/// Returns the CLI's stdout (a short success line) on success.
pub fn install_plugin(install_id: &str, scope: &str) -> AppResult<String> {
    if install_id.trim().is_empty() {
        return Err(AppError::InvalidArgument("empty plugin id".into()));
    }
    // install_id comes from our own catalogue; validate defensively before it
    // reaches a subprocess argument.
    if !install_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | '/'))
    {
        return Err(AppError::InvalidArgument(format!("invalid plugin id: {install_id}")));
    }
    let scope = match scope {
        "user" | "project" | "local" => scope,
        _ => "user",
    };
    let out = run_claude(&["plugin", "install", install_id, "--scope", scope])?;
    Ok(out.trim().to_string())
}
