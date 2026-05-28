use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Default marketplace location. Resolved with /raw.githubusercontent.com so we
/// avoid HTML scraping. The file is expected to be a JSON array of MarketplacePlugin.
/// If the URL 404s or the network is unavailable we fall back to a small bundled list.
const DEFAULT_MARKETPLACE_URL: &str =
    "https://raw.githubusercontent.com/anthropics/claude-code-plugins/main/marketplace.json";

const CACHE_FILE: &str = "plugins-cache.json";
const CACHE_TTL_SECS: u64 = 60 * 60 * 24; // 24 hours

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    pub name: String,
    pub slug: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePlugin {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub repo_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceSnapshot {
    pub source: String,
    pub fetched_at_ms: u64,
    pub is_fallback: bool,
    pub plugins: Vec<MarketplacePlugin>,
}

pub fn list_installed() -> Vec<InstalledPlugin> {
    let Some(home) = dirs::home_dir() else { return Vec::new(); };
    let root = home.join(".claude").join("plugins");
    let Ok(entries) = fs::read_dir(&root) else { return Vec::new(); };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() { continue; }
        let slug = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if slug.is_empty() || slug.starts_with('.') { continue; }
        let (name, version, description) = read_plugin_manifest(&p);
        out.push(InstalledPlugin {
            name: name.unwrap_or_else(|| slug.clone()),
            slug,
            version,
            description,
            path: p.display().to_string(),
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn read_plugin_manifest(dir: &Path) -> (Option<String>, Option<String>, Option<String>) {
    for candidate in &["plugin.json", "manifest.json", "package.json"] {
        let p = dir.join(candidate);
        let Ok(content) = fs::read_to_string(&p) else { continue; };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else { continue; };
        let name = v.get("name").and_then(|x| x.as_str()).map(String::from);
        let version = v.get("version").and_then(|x| x.as_str()).map(String::from);
        let description = v.get("description").and_then(|x| x.as_str()).map(String::from);
        return (name, version, description);
    }
    (None, None, None)
}

pub async fn fetch_marketplace(force: bool) -> AppResult<MarketplaceSnapshot> {
    let cache_path = cache_file_path()?;
    if !force {
        if let Some(cached) = read_cache(&cache_path) {
            if !is_stale(cached.fetched_at_ms) { return Ok(cached); }
        }
    }
    let url = DEFAULT_MARKETPLACE_URL;
    match fetch_remote(url).await {
        Ok(plugins) => {
            let snap = MarketplaceSnapshot {
                source: url.into(),
                fetched_at_ms: now_ms(),
                is_fallback: false,
                plugins,
            };
            let _ = write_cache(&cache_path, &snap);
            Ok(snap)
        }
        Err(_) => {
            // Network/parse error → return cached if any, else the bundled fallback.
            if let Some(cached) = read_cache(&cache_path) {
                return Ok(cached);
            }
            let snap = MarketplaceSnapshot {
                source: "bundled fallback".into(),
                fetched_at_ms: now_ms(),
                is_fallback: true,
                plugins: fallback_list(),
            };
            Ok(snap)
        }
    }
}

async fn fetch_remote(url: &str) -> AppResult<Vec<MarketplacePlugin>> {
    let body = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| AppError::Other(format!("http client: {e}")))?
        .get(url)
        .header("User-Agent", "agent-console")
        .send()
        .await
        .map_err(|e| AppError::Other(format!("fetch failed: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Other(format!("status: {e}")))?
        .text()
        .await
        .map_err(|e| AppError::Other(format!("read body: {e}")))?;
    let plugins: Vec<MarketplacePlugin> = serde_json::from_str(&body)
        .map_err(|e| AppError::Other(format!("parse: {e}")))?;
    Ok(plugins)
}

fn fallback_list() -> Vec<MarketplacePlugin> {
    // Tiny starter set so the panel is never empty when the marketplace is
    // unreachable. Slugs are illustrative — confirm before installing.
    vec![
        MarketplacePlugin {
            name: "claude-code-companion".into(),
            slug: "claude-code-companion".into(),
            description: "Curated helpers and slash commands for everyday Claude Code use.".into(),
            author: Some("community".into()),
            repo_url: Some("https://github.com/anthropics/claude-code".into()),
            tags: vec!["starter".into(), "general".into()],
        },
        MarketplacePlugin {
            name: "git-power-tools".into(),
            slug: "git-power-tools".into(),
            description: "Slash commands and hooks for richer git workflows inside Claude Code.".into(),
            author: None,
            repo_url: None,
            tags: vec!["git".into()],
        },
        MarketplacePlugin {
            name: "test-runner".into(),
            slug: "test-runner".into(),
            description: "Detect and run the project's test suite, summarising failures.".into(),
            author: None,
            repo_url: None,
            tags: vec!["testing".into()],
        },
    ]
}

fn cache_file_path() -> AppResult<PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| AppError::Other("no data dir".into()))?
        .join("agent-console");
    fs::create_dir_all(&dir).map_err(AppError::Io)?;
    Ok(dir.join(CACHE_FILE))
}

fn read_cache(p: &Path) -> Option<MarketplaceSnapshot> {
    let s = fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_cache(p: &Path, snap: &MarketplaceSnapshot) -> AppResult<()> {
    let s = serde_json::to_string_pretty(snap)
        .map_err(|e| AppError::Other(format!("serialize cache: {e}")))?;
    fs::write(p, s).map_err(AppError::Io)
}

fn is_stale(ts_ms: u64) -> bool {
    let now = now_ms();
    now.saturating_sub(ts_ms) / 1000 > CACHE_TTL_SECS
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
