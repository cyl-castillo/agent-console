//! Jira integration: see the issues assigned to you and turn one into an
//! agent session. Read-mostly by design.
//!
//! - Sites: any number, one active at a time (a company Jira and a client's,
//!   say). The site URL + email/username of each live in a small global JSON
//!   config; each site's secret lives in the OS keychain under its own
//!   account (never in the JSON, never in a log). The secret only ever
//!   leaves the machine as an auth header to that site.
//! - Cloud (`*.atlassian.net`, REST v3): Basic auth with the Atlassian API
//!   token. The legacy `/rest/api/3/search` was removed, so we use
//!   `POST /rest/api/3/search/jql`.
//! - Server / Data Center (self-hosted, REST v2, plain http allowed for an
//!   intranet): a personal access token as Bearer, or a username + password as
//!   Basic — whichever `/myself` accepts at connect time is remembered.

use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

const KEYRING_SERVICE: &str = "agent-console:jira";
/// The single-site era stored the one token here; migrated to a per-site
/// account on first load (see `load_sites`).
const LEGACY_KEYRING_ACCOUNT: &str = "api-token";

/// Cloud or self-hosted: decides the REST version, the auth styles that can
/// work and the worklog comment format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JiraKind {
    #[default]
    Cloud,
    Server,
}

/// How the stored secret is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JiraAuth {
    /// Cloud: Basic `email:api-token`.
    #[default]
    ApiToken,
    /// Server/DC: `Authorization: Bearer <personal access token>`.
    Pat,
    /// Server/DC: Basic `username:password`.
    Password,
}

/// One connected site (the non-secret half).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraSite {
    /// Stable id derived from the host; also the keychain account suffix.
    pub id: String,
    /// Base URL, e.g. `https://acme.atlassian.net` or `http://jira.corp.local`.
    pub site_url: String,
    /// Email (Cloud) or username (Server/DC).
    pub email: String,
    #[serde(default)]
    pub kind: JiraKind,
    #[serde(default)]
    pub auth: JiraAuth,
}

/// The config file: every site and which one the panel shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraSites {
    pub sites: Vec<JiraSite>,
    pub active: Option<String>,
}

/// The non-secret half of the connection, persisted as JSON.
/// The single-site config file of v0.75–v0.94 (`{siteUrl, email}`), read
/// once and migrated into `JiraSites`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraConfig {
    /// Cloud site base, e.g. `https://acme.atlassian.net`.
    pub site_url: String,
    pub email: String,
}

/// What the UI needs to decide "connect form" vs "issue list", without ever
/// shipping a secret to the frontend. `site_url`/`email` describe the active
/// site; `sites` lists every connected one for the switcher.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraStatus {
    pub configured: bool,
    pub site_url: String,
    pub email: String,
    pub active_id: Option<String>,
    pub sites: Vec<JiraSiteInfo>,
}

/// One row of the site switcher.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraSiteInfo {
    pub id: String,
    pub site_url: String,
    pub email: String,
    pub kind: JiraKind,
    /// The keychain still holds this site's secret.
    pub connected: bool,
}

/// One assigned issue, flattened from Jira's nested fields for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssue {
    pub key: String,
    pub summary: String,
    pub status: String,
    /// statusCategory key: "new" | "indeterminate" | "done".
    pub status_category: String,
    pub priority: Option<String>,
    pub issue_type: String,
    pub due_date: Option<String>,
    pub project: String,
    pub updated: Option<String>,
    /// Human browse URL, derived from the site + key.
    pub url: String,
}

fn config_path() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("jira.json"))
}

fn keyring_entry(account: &str) -> AppResult<Entry> {
    Entry::new(KEYRING_SERVICE, account).map_err(|e| AppError::Other(format!("keyring open: {e}")))
}

/// Keychain account for a site's secret.
fn secret_account(site_id: &str) -> String {
    format!("secret:{site_id}")
}

fn get_secret(site_id: &str) -> Option<String> {
    keyring_entry(&secret_account(site_id))
        .ok()?
        .get_password()
        .ok()
}

fn set_secret(site_id: &str, secret: &str) -> AppResult<()> {
    keyring_entry(&secret_account(site_id))?
        .set_password(secret)
        .map_err(|e| AppError::Other(format!("keyring set: {e}")))
}

fn delete_secret(site_id: &str) {
    if let Ok(entry) = keyring_entry(&secret_account(site_id)) {
        let _ = entry.delete_credential();
    }
}

/// Parse the config file in either shape: the current `{sites, active}` or
/// the single-site `{siteUrl, email}` of earlier versions, which becomes one
/// Cloud site (the only kind that existed) that is active.
pub fn parse_sites(text: &str) -> JiraSites {
    if let Ok(v) = serde_json::from_str::<JiraSites>(text) {
        if !v.sites.is_empty() || text.contains("\"sites\"") {
            return v;
        }
    }
    match serde_json::from_str::<JiraConfig>(text) {
        Ok(old) if !old.site_url.is_empty() => {
            let id = site_id(&old.site_url);
            JiraSites {
                active: Some(id.clone()),
                sites: vec![JiraSite {
                    id,
                    site_url: old.site_url,
                    email: old.email,
                    kind: JiraKind::Cloud,
                    auth: JiraAuth::ApiToken,
                }],
            }
        }
        _ => JiraSites::default(),
    }
}

/// Read the sites; a legacy single-site file is migrated on the spot, token
/// included (moved from the old keychain account to the site's own).
pub fn load_sites() -> JiraSites {
    let Ok(path) = config_path() else {
        return JiraSites::default();
    };
    let Ok(txt) = std::fs::read_to_string(&path) else {
        return JiraSites::default();
    };
    let sites = parse_sites(&txt);
    let legacy = !txt.contains("\"sites\"");
    if legacy && !sites.sites.is_empty() {
        let id = &sites.sites[0].id;
        if let Ok(old) = keyring_entry(LEGACY_KEYRING_ACCOUNT) {
            if let Ok(tok) = old.get_password() {
                if set_secret(id, &tok).is_ok() {
                    let _ = old.delete_credential();
                }
            }
        }
        let _ = save_sites(&sites);
    }
    sites
}

fn save_sites(sites: &JiraSites) -> AppResult<()> {
    let json = serde_json::to_string_pretty(sites).map_err(|e| AppError::Other(e.to_string()))?;
    std::fs::write(config_path()?, json)?;
    Ok(())
}

/// The site the panel shows.
pub fn active_site() -> Option<JiraSite> {
    let all = load_sites();
    let id = all.active.clone()?;
    all.sites.into_iter().find(|s| s.id == id)
}

/// A stable, filename- and keychain-safe id from the site's host:
/// `https://acme.atlassian.net` → `acme-atlassian-net`.
pub fn site_id(site_url: &str) -> String {
    let host = site_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let mut out = String::with_capacity(host.len());
    for c in host.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Cloud lives on `*.atlassian.net`; anything else is self-hosted. (A Cloud
/// custom domain is rare enough that the connect probe below decides.)
pub fn kind_from_host(site_url: &str) -> JiraKind {
    let host = site_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("");
    if host.ends_with(".atlassian.net") {
        JiraKind::Cloud
    } else {
        JiraKind::Server
    }
}

/// Normalize/validate the site URL: a host with an http(s) scheme (none
/// given → https). Plain http is allowed because a self-hosted Jira on an
/// intranet often has no certificate; the panel says so. Trailing slash
/// trimmed so we can concatenate `/rest/...` cleanly.
pub fn normalize_site(raw: &str) -> AppResult<String> {
    let s = raw.trim().trim_end_matches('/');
    let s = if s.starts_with("https://") || s.starts_with("http://") {
        s.to_string()
    } else if s.contains("://") {
        return Err(AppError::InvalidArgument(
            "site URL must start with https:// or http://".into(),
        ));
    } else {
        format!("https://{s}")
    };
    let host = s.split("://").nth(1).unwrap_or("");
    if host.len() < "a.b".len() || !host.contains('.') {
        return Err(AppError::InvalidArgument(
            "site URL needs a host (e.g. https://yourteam.atlassian.net or http://jira.yourcompany.local)"
                .into(),
        ));
    }
    // No spaces or control chars — it becomes a request URL.
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AppError::InvalidArgument(
            "site URL contains invalid characters".into(),
        ));
    }
    Ok(s)
}

pub fn status() -> JiraStatus {
    let all = load_sites();
    let infos: Vec<JiraSiteInfo> = all
        .sites
        .iter()
        .map(|s| JiraSiteInfo {
            id: s.id.clone(),
            site_url: s.site_url.clone(),
            email: s.email.clone(),
            kind: s.kind,
            connected: get_secret(&s.id).is_some(),
        })
        .collect();
    let active = all
        .active
        .as_ref()
        .and_then(|id| infos.iter().find(|s| &s.id == id));
    JiraStatus {
        configured: active.is_some_and(|s| s.connected && !s.email.is_empty()),
        site_url: active.map(|s| s.site_url.clone()).unwrap_or_default(),
        email: active.map(|s| s.email.clone()).unwrap_or_default(),
        active_id: active.map(|s| s.id.clone()),
        sites: infos,
    }
}

/// What the connect form can fill in before the user types anything.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JiraDefaults {
    /// `git config user.email` (the project's, else the global one).
    pub email: Option<String>,
    /// The first `https://<team>.atlassian.net` the project mentions (README,
    /// PR templates, docs…) — the site is almost always written down somewhere.
    pub site_url: Option<String>,
}

/// Detect instead of ask: the two Jira fields that the machine already knows.
/// Never fails — a missing git or a project without a Jira mention just
/// leaves the field empty.
pub fn defaults(project_root: Option<&std::path::Path>) -> JiraDefaults {
    let mut git = std::process::Command::new("git");
    if let Some(root) = project_root {
        git.current_dir(root);
    }
    let email = git
        .args(["config", "--get", "user.email"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|e| e.contains('@'));
    let site_url = project_root.and_then(|root| {
        std::process::Command::new("git")
            .current_dir(root)
            .args([
                "grep",
                "-I",
                "-o",
                "-h",
                "-E",
                "--max-count=1",
                r"https://[a-z0-9-]+\.atlassian\.net",
                "--",
                "*.md",
                "*.txt",
                "*.json",
                "*.yml",
                "*.yaml",
                ".github",
            ])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| first_atlassian_site(&String::from_utf8_lossy(&o.stdout)))
    });
    JiraDefaults { email, site_url }
}

/// The first Atlassian cloud site in a blob of text, as a base URL.
pub fn first_atlassian_site(text: &str) -> Option<String> {
    let idx = text.find("https://")?;
    let rest = &text[idx + "https://".len()..];
    let host: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.')
        .collect();
    let host = host.trim_end_matches('.');
    if host.ends_with(".atlassian.net") && host.len() > ".atlassian.net".len() {
        Some(format!("https://{host}"))
    } else {
        None
    }
}

/// Validate the connect form's three fields; the normalized site and the
/// trimmed email/secret come back.
pub fn validate_inputs(
    site_url: &str,
    email: &str,
    secret: &str,
) -> AppResult<(String, String, String)> {
    let site = normalize_site(site_url)?;
    let email = email.trim();
    if email.is_empty() {
        return Err(AppError::InvalidArgument(
            "email (Cloud) or username (Server) is required".into(),
        ));
    }
    let secret = secret.trim();
    if secret.is_empty() {
        return Err(AppError::InvalidArgument(
            "an API token, personal access token or password is required".into(),
        ));
    }
    Ok((site, email.to_string(), secret.to_string()))
}

/// Add (or update) a site and make it active, validating the credentials
/// first. Cloud: Basic email + API token against REST v3. Self-hosted: the
/// secret is tried as a personal access token (Bearer), then as a password
/// (Basic) — whichever `/myself` accepts is what gets remembered. Nothing is
/// persisted until the site answered. Returns the account's display name.
pub async fn connect(site_url: &str, email: &str, secret: &str) -> AppResult<String> {
    let (site_url, email, secret) = validate_inputs(site_url, email, secret)?;
    let kind = detect_kind(&site_url).await;
    let candidates: &[JiraAuth] = match kind {
        JiraKind::Cloud => &[JiraAuth::ApiToken],
        JiraKind::Server => &[JiraAuth::Pat, JiraAuth::Password],
    };
    let mut last_err = None;
    for auth in candidates {
        let probe = JiraSite {
            id: site_id(&site_url),
            site_url: site_url.clone(),
            email: email.clone(),
            kind,
            auth: *auth,
        };
        match myself(&probe, &secret).await {
            Ok(name) => {
                set_secret(&probe.id, &secret)?;
                let mut all = load_sites();
                all.sites.retain(|s| s.id != probe.id);
                all.active = Some(probe.id.clone());
                all.sites.push(probe);
                save_sites(&all)?;
                return Ok(name);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| AppError::Other("Jira did not answer".into())))
}

/// Show another connected site.
pub fn switch(site_id: &str) -> AppResult<()> {
    let mut all = load_sites();
    if !all.sites.iter().any(|s| s.id == site_id) {
        return Err(AppError::InvalidArgument("unknown Jira site".into()));
    }
    all.active = Some(site_id.to_string());
    save_sites(&all)
}

/// Remove one site: its secret and its row. The active one moves to the
/// next site left, if any.
pub fn forget(site_id: &str) -> AppResult<()> {
    let mut all = load_sites();
    delete_secret(site_id);
    all.sites.retain(|s| s.id != site_id);
    if all.active.as_deref() == Some(site_id) {
        all.active = all.sites.first().map(|s| s.id.clone());
    }
    if all.sites.is_empty() {
        if let Ok(path) = config_path() {
            let _ = std::fs::remove_file(path);
        }
        if let Ok(old) = keyring_entry(LEGACY_KEYRING_ACCOUNT) {
            let _ = old.delete_credential();
        }
        return Ok(());
    }
    save_sites(&all)
}

/// Forget the active site (the panel's "disconnect"); other sites stay.
pub fn disconnect() -> AppResult<()> {
    match load_sites().active {
        Some(id) => forget(&id),
        None => Ok(()),
    }
}

/// `GET /rest/api/2/serverInfo` is public on both deployments and says which
/// one answers (`deploymentType`). Unreachable → judge by the host.
async fn detect_kind(site_url: &str) -> JiraKind {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Info {
        deployment_type: Option<String>,
    }
    let probe = async {
        let resp = client()
            .ok()?
            .get(format!("{site_url}/rest/api/2/serverInfo"))
            .header("Accept", "application/json")
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let info: Info = resp.json().await.ok()?;
        info.deployment_type
    };
    match probe.await.as_deref() {
        Some("Cloud") => JiraKind::Cloud,
        Some(_) => JiraKind::Server,
        None => kind_from_host(site_url),
    }
}

/// REST version per deployment: Cloud removed v2 for search; Server/DC has no v3.
pub fn api_version(kind: JiraKind) -> &'static str {
    match kind {
        JiraKind::Cloud => "3",
        JiraKind::Server => "2",
    }
}

/// The search endpoint per deployment.
pub fn search_path(kind: JiraKind) -> &'static str {
    match kind {
        JiraKind::Cloud => "/rest/api/3/search/jql",
        JiraKind::Server => "/rest/api/2/search",
    }
}

/// Attach the site's credentials the way it wants them.
fn authed(req: reqwest::RequestBuilder, site: &JiraSite, secret: &str) -> reqwest::RequestBuilder {
    match site.auth {
        JiraAuth::Pat => req.bearer_auth(secret),
        JiraAuth::ApiToken | JiraAuth::Password => req.basic_auth(&site.email, Some(secret)),
    }
}

/// `GET /myself` with the given site + secret: the display name on success.
async fn myself(site: &JiraSite, secret: &str) -> AppResult<String> {
    let req = client()?
        .get(format!(
            "{}/rest/api/{}/myself",
            site.site_url,
            api_version(site.kind)
        ))
        .header("Accept", "application/json");
    let resp = authed(req, site, secret)
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Jira request failed: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Other(match site.kind {
            JiraKind::Cloud => {
                "Jira rejected the credentials (401) — check the email and API token".into()
            }
            JiraKind::Server => {
                "Jira rejected the credentials (401) — a personal access token, or your username and password"
                    .into()
            }
        }));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Other(format!(
            "Jira returned {status}{}",
            error_detail(&body)
        )));
    }
    let me: MyselfResp = resp
        .json()
        .await
        .map_err(|e| AppError::Other(format!("bad Jira response: {e}")))?;
    Ok(me.display_name.unwrap_or_else(|| site.email.clone()))
}

fn client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("agent-console")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Other(format!("http client: {e}")))
}

/// The active site and its secret.
fn creds() -> AppResult<(JiraSite, String)> {
    let site = active_site()
        .filter(|s| !s.site_url.is_empty() && !s.email.is_empty())
        .ok_or_else(|| AppError::InvalidArgument("Jira is not configured".into()))?;
    let secret = get_secret(&site.id)
        .ok_or_else(|| AppError::InvalidArgument("Jira secret missing from keychain".into()))?;
    Ok((site, secret))
}

const DEFAULT_JQL: &str = "assignee = currentUser() AND statusCategory != Done ORDER BY duedate ASC, priority DESC, updated DESC";

/// The issue fields the queue reads; the same list on both deployments.
const SEARCH_FIELDS: [&str; 7] = [
    "summary",
    "status",
    "priority",
    "duedate",
    "issuetype",
    "project",
    "updated",
];

/// Jira's own words for a failed request — `errorMessages` and `errors` of
/// its JSON error body, or a short tail of a non-JSON one — so a 400 says
/// *why* ("Field 'sprint' does not exist") instead of just the number.
pub fn error_detail(body: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(arr) = v.get("errorMessages").and_then(|m| m.as_array()) {
            parts.extend(arr.iter().filter_map(|m| m.as_str()).map(str::to_string));
        }
        if let Some(obj) = v.get("errors").and_then(|e| e.as_object()) {
            parts.extend(
                obj.iter()
                    .filter_map(|(k, val)| val.as_str().map(|s| format!("{k}: {s}"))),
            );
        }
        if parts.is_empty() {
            if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
                parts.push(m.to_string());
            }
        }
    } else {
        let plain: String = body
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        if !plain.is_empty() && !plain.starts_with('<') {
            parts.push(plain);
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    let joined = parts.join(" · ");
    let cut: String = joined.chars().take(300).collect();
    format!(
        " — {cut}{}",
        if cut.len() < joined.len() { "…" } else { "" }
    )
}

/// Guard a caller-supplied JQL before it becomes a request body: length-capped
/// and free of control characters. Jira itself is the real parser — this only
/// keeps junk and log-breaking bytes out.
fn validate_jql(jql: &str) -> AppResult<&str> {
    let j = jql.trim();
    if j.is_empty() {
        return Err(AppError::InvalidArgument("JQL is empty".into()));
    }
    if j.len() > 1000 {
        return Err(AppError::InvalidArgument("JQL is too long".into()));
    }
    if j.chars().any(|c| c.is_control()) {
        return Err(AppError::InvalidArgument(
            "JQL contains control characters".into(),
        ));
    }
    Ok(j)
}

/// Issues for the given JQL (role presets / user-tuned), or the classic
/// "assigned to me, not Done" when none is provided.
pub async fn list_assigned(jql: Option<&str>) -> AppResult<Vec<JiraIssue>> {
    let (site, secret) = creds()?;
    let jql = match jql {
        Some(j) => validate_jql(j)?,
        None => DEFAULT_JQL,
    };
    let req = match site.kind {
        // Cloud: the only search left is POST /search/jql with a JSON body.
        JiraKind::Cloud => client()?
            .post(format!("{}{}", site.site_url, search_path(site.kind)))
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "jql": jql,
                "maxResults": 50,
                "fields": SEARCH_FIELDS,
            })),
        // Server/DC: GET with query parameters is what every version (and
        // every reverse proxy in front of one) accepts; a JSON POST to
        // /rest/api/2/search came back 400 from a self-hosted instance.
        JiraKind::Server => client()?
            .get(format!("{}{}", site.site_url, search_path(site.kind)))
            .header("Accept", "application/json")
            .query(&[
                ("jql", jql),
                ("maxResults", "50"),
                ("fields", &SEARCH_FIELDS.join(",")),
            ]),
    };
    let resp = authed(req, &site, &secret)
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Jira request failed: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Other(
            "Jira rejected the credentials (401)".into(),
        ));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Other(format!(
            "Jira search returned {status}{}",
            error_detail(&body)
        )));
    }
    let parsed: SearchResp = resp
        .json()
        .await
        .map_err(|e| AppError::Other(format!("bad Jira response: {e}")))?;
    Ok(parsed
        .issues
        .into_iter()
        .map(|i| flatten(&site.site_url, i))
        .collect())
}

/// Parse a human duration into seconds: "1h 30m", "1h30m", "90m", "2h",
/// "1.5h", or a bare number (minutes). Rejects zero, negatives and garbage —
/// this feeds a billing-adjacent API, better to ask again than to guess.
pub fn parse_duration_to_seconds(input: &str) -> AppResult<u64> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return Err(AppError::InvalidArgument("empty duration".into()));
    }
    // Bare number = minutes ("45" → 45m).
    if let Ok(mins) = s.parse::<f64>() {
        return finish_duration(mins * 60.0);
    }
    let mut total = 0.0f64;
    let mut rest = s.as_str();
    for unit_secs in [("h", 3600.0), ("m", 60.0)] {
        let (unit, secs_per) = unit_secs;
        if let Some(pos) = rest.find(unit) {
            let (num_part, tail) = rest.split_at(pos);
            let num = num_part.trim();
            if !num.is_empty() {
                let v: f64 = num
                    .parse()
                    .map_err(|_| AppError::InvalidArgument(format!("bad duration near '{num}'")))?;
                if v < 0.0 {
                    return Err(AppError::InvalidArgument(
                        "duration must be positive".into(),
                    ));
                }
                total += v * secs_per;
            }
            rest = tail[unit.len()..].trim_start();
        }
    }
    if !rest.trim().is_empty() {
        return Err(AppError::InvalidArgument(format!(
            "couldn't parse duration '{input}' — use forms like \"1h 30m\", \"90m\", \"2h\""
        )));
    }
    finish_duration(total)
}

/// Shared tail: round, and reject non-finite, zero/negative, or absurd
/// (> 999h) durations before they reach a billing-adjacent API.
fn finish_duration(total_secs: f64) -> AppResult<u64> {
    let secs = total_secs.round();
    if !secs.is_finite() || secs <= 0.0 {
        return Err(AppError::InvalidArgument(
            "duration must be positive".into(),
        ));
    }
    if secs > 999.0 * 3600.0 {
        return Err(AppError::InvalidArgument(
            "duration is implausibly large".into(),
        ));
    }
    Ok(secs as u64)
}

/// Seconds → Jira-style compact label ("1h 30m", "45m").
pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600 + 30) / 60; // round to the minute for display
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

fn valid_issue_key(key: &str) -> bool {
    let mut parts = key.splitn(2, '-');
    let (Some(proj), Some(num)) = (parts.next(), parts.next()) else {
        return false;
    };
    !proj.is_empty()
        && proj.chars().all(|c| c.is_ascii_alphanumeric())
        && proj.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
}

fn valid_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && d.chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

/// Log work on an issue. `duration` is human ("1h 30m"); `started_date` is
/// YYYY-MM-DD (logged at 12:00 local-agnostic to dodge timezone day flips);
/// `comment` optional. Returns the label actually logged ("1h 30m").
pub async fn log_work(
    issue_key: &str,
    duration: &str,
    started_date: &str,
    comment: Option<&str>,
) -> AppResult<String> {
    let key = issue_key.trim().to_uppercase();
    if !valid_issue_key(&key) {
        return Err(AppError::InvalidArgument(format!(
            "'{issue_key}' doesn't look like an issue key"
        )));
    }
    if !valid_date(started_date) {
        return Err(AppError::InvalidArgument(
            "started date must be YYYY-MM-DD".into(),
        ));
    }
    let secs = parse_duration_to_seconds(duration)?;

    let (site, secret) = creds()?;
    let body = worklog_body(site.kind, secs, started_date, comment);
    let req = client()?
        .post(format!(
            "{}/rest/api/{}/issue/{key}/worklog",
            site.site_url,
            api_version(site.kind)
        ))
        .header("Accept", "application/json")
        .json(&body);
    let resp = authed(req, &site, &secret)
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Jira request failed: {e}")))?;
    match resp.status() {
        s if s.is_success() => Ok(format_duration(secs)),
        reqwest::StatusCode::UNAUTHORIZED => Err(AppError::Other(
            "Jira rejected the credentials (401)".into(),
        )),
        reqwest::StatusCode::NOT_FOUND => Err(AppError::Other(format!(
            "Jira says {key} doesn't exist (404)"
        ))),
        s => {
            let body = resp.text().await.unwrap_or_default();
            Err(AppError::Other(format!(
                "Jira worklog returned {s}{}",
                error_detail(&body)
            )))
        }
    }
}

/// The worklog request body: Cloud's v3 wants the comment as an ADF document,
/// Server's v2 wants plain text.
pub fn worklog_body(
    kind: JiraKind,
    secs: u64,
    started_date: &str,
    comment: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "timeSpentSeconds": secs,
        "started": format!("{started_date}T12:00:00.000+0000"),
    });
    if let Some(text) = comment.map(str::trim).filter(|t| !t.is_empty()) {
        body["comment"] = match kind {
            JiraKind::Cloud => serde_json::json!({
                "type": "doc",
                "version": 1,
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }],
            }),
            JiraKind::Server => serde_json::Value::String(text.to_string()),
        };
    }
    body
}

/// Estimate seconds actually worked from witnessed event timestamps (the
/// Testigo ledger events of a `jira:<KEY>` case): sort, merge events closer
/// than `gap_ms` into one burst, and total the burst spans. A burst never
/// counts less than `min_burst_ms` — a lone prompt still took real time.
/// Deliberately an ESTIMATE feeding a human-editable field, never auto-submitted.
pub fn estimate_worked_seconds(mut ts_ms: Vec<i64>, gap_ms: i64, min_burst_ms: i64) -> u64 {
    ts_ms.retain(|t| *t > 0);
    if ts_ms.is_empty() {
        return 0;
    }
    ts_ms.sort_unstable();
    let mut total_ms: i64 = 0;
    let mut burst_start = ts_ms[0];
    let mut prev = ts_ms[0];
    for &t in &ts_ms[1..] {
        if t - prev > gap_ms {
            total_ms += (prev - burst_start).max(min_burst_ms);
            burst_start = t;
        }
        prev = t;
    }
    total_ms += (prev - burst_start).max(min_burst_ms);
    (total_ms / 1000).max(0) as u64
}

// ---------------------------------------------------------------------------
// Daily worklog digest: which tickets saw witnessed activity on a given local
// day, how long, and whether their time was already logged. The estimation is
// the same burst-clustering the per-ticket suggestion uses; the "already
// logged" marks live in a small per-project JSON so the digest never nags
// about (or double-logs) a (ticket, day) that's been submitted.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorklogMark {
    pub issue_key: String,
    /// Local day the work happened (YYYY-MM-DD), NOT when it was logged.
    pub date: String,
    pub seconds: u64,
    pub logged_at_ms: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarksFile {
    #[serde(default)]
    marks: Vec<WorklogMark>,
}

fn marks_path(project_root: &str) -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console")
        .join("worklog-marks");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!(
        "{}.json",
        crate::services::persistence::project_file_key(project_root)
    )))
}

pub fn load_marks(project_root: &str) -> Vec<WorklogMark> {
    let Ok(path) = marks_path(project_root) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<MarksFile>(&raw).ok())
        .map(|f| f.marks)
        .unwrap_or_default()
}

/// Record that (issue, day) was logged. Idempotent by (issue_key, date):
/// re-logging the same day replaces the mark.
pub fn add_mark(project_root: &str, mark: WorklogMark) -> AppResult<()> {
    let path = marks_path(project_root)?;
    let mut marks = load_marks(project_root);
    marks.retain(|m| !(m.issue_key == mark.issue_key && m.date == mark.date));
    marks.push(mark);
    let raw = serde_json::to_string_pretty(&MarksFile { marks })
        .map_err(|e| AppError::Other(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestEntry {
    pub issue_key: String,
    pub seconds: u64,
    pub events: usize,
    /// Already submitted for this (ticket, day) — shown as done, never re-logged.
    pub logged: bool,
    pub logged_seconds: Option<u64>,
}

/// Build the digest for one local day from witnessed (case_id, ts) pairs.
/// Pure — the command layer feeds it from the Testigo ledger. Entries under a
/// minute are noise, not work.
pub fn build_daily_digest(
    case_events: &[(String, i64)],
    day_start_ms: i64,
    day_end_ms: i64,
    date: &str,
    marks: &[WorklogMark],
) -> Vec<DigestEntry> {
    use std::collections::BTreeMap;
    let mut by_key: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (case_id, ts) in case_events {
        if *ts < day_start_ms || *ts >= day_end_ms {
            continue;
        }
        let Some(key) = case_id.strip_prefix("jira:") else {
            continue;
        };
        by_key.entry(key.to_string()).or_default().push(*ts);
    }
    let mut out: Vec<DigestEntry> = Vec::new();
    for (key, ts_list) in by_key {
        let events = ts_list.len();
        let seconds = parse_free_seconds(ts_list);
        if seconds < 60 {
            continue;
        }
        let mark = marks.iter().find(|m| m.issue_key == key && m.date == date);
        out.push(DigestEntry {
            issue_key: key,
            seconds,
            events,
            logged: mark.is_some(),
            logged_seconds: mark.map(|m| m.seconds),
        });
    }
    // Biggest chunks of the day first.
    out.sort_by_key(|e| std::cmp::Reverse(e.seconds));
    out
}

/// Burst-clustering with the digest's standard knobs (15min gap, 1min floor).
fn parse_free_seconds(ts: Vec<i64>) -> u64 {
    estimate_worked_seconds(ts, 15 * 60 * 1000, 60 * 1000)
}

fn flatten(site: &str, i: RawIssue) -> JiraIssue {
    let f = i.fields;
    JiraIssue {
        url: format!("{site}/browse/{}", i.key),
        key: i.key,
        summary: f.summary.unwrap_or_default(),
        status: f
            .status
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default(),
        status_category: f
            .status
            .and_then(|s| s.status_category)
            .map(|c| c.key)
            .unwrap_or_else(|| "new".into()),
        priority: f.priority.map(|p| p.name),
        issue_type: f.issuetype.map(|t| t.name).unwrap_or_default(),
        due_date: f.duedate,
        project: f.project.map(|p| p.name).unwrap_or_default(),
        updated: f.updated,
    }
}

// --- Jira response shapes (only the fields we read) ---

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MyselfResp {
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct SearchResp {
    #[serde(default)]
    issues: Vec<RawIssue>,
}

#[derive(Deserialize)]
struct RawIssue {
    key: String,
    fields: RawFields,
}

#[derive(Deserialize)]
struct RawFields {
    summary: Option<String>,
    status: Option<RawStatus>,
    priority: Option<RawNamed>,
    duedate: Option<String>,
    issuetype: Option<RawNamed>,
    project: Option<RawNamed>,
    updated: Option<String>,
}

#[derive(Deserialize)]
struct RawStatus {
    name: String,
    #[serde(rename = "statusCategory")]
    status_category: Option<RawStatusCategory>,
}

#[derive(Deserialize)]
struct RawStatusCategory {
    key: String,
}

#[derive(Deserialize)]
struct RawNamed {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_atlassian_site_reads_the_host_and_nothing_else() {
        assert_eq!(
            first_atlassian_site("see https://acme.atlassian.net/browse/ACME-12 and more"),
            Some("https://acme.atlassian.net".into())
        );
        assert_eq!(
            first_atlassian_site("https://acme-team.atlassian.net.\nhttps://other.atlassian.net"),
            Some("https://acme-team.atlassian.net".into())
        );
        assert_eq!(first_atlassian_site("https://example.com/x"), None);
        assert_eq!(first_atlassian_site("https://.atlassian.net"), None);
        assert_eq!(first_atlassian_site("no links"), None);
    }

    #[test]
    fn defaults_never_fail_without_a_project() {
        // Whatever git says on this machine, the call returns a value.
        let d = defaults(None);
        if let Some(e) = &d.email {
            assert!(e.contains('@'));
        }
        assert!(d.site_url.is_none());
        let none = defaults(Some(std::path::Path::new("/definitely/not/a/repo")));
        assert!(none.site_url.is_none());
    }

    #[test]
    fn normalize_site_accepts_http_and_bare_hosts_and_trims() {
        assert_eq!(
            normalize_site("https://acme.atlassian.net/").unwrap(),
            "https://acme.atlassian.net"
        );
        // A self-hosted Jira on an intranet, plain http — the BSE case.
        assert_eq!(
            normalize_site(" http://jiraportal.bse.com.uy/ ").unwrap(),
            "http://jiraportal.bse.com.uy"
        );
        assert_eq!(
            normalize_site("acme.atlassian.net").unwrap(),
            "https://acme.atlassian.net"
        );
        assert!(normalize_site("ftp://acme.atlassian.net").is_err());
        assert!(normalize_site("https://a b.net").is_err());
        assert!(normalize_site("https://localhost").is_err());
        assert!(normalize_site("").is_err());
    }

    #[test]
    fn validate_inputs_rejects_blank_identity_and_secret() {
        assert!(validate_inputs("https://x.atlassian.net", "  ", "tok").is_err());
        assert!(validate_inputs("https://x.atlassian.net", "me@x.com", "  ").is_err());
        assert_eq!(
            validate_inputs("http://jira.corp/", " jdoe ", " pat-1 ").unwrap(),
            ("http://jira.corp".into(), "jdoe".into(), "pat-1".into())
        );
    }

    #[test]
    fn site_id_and_kind_follow_the_host() {
        assert_eq!(site_id("https://acme.atlassian.net"), "acme-atlassian-net");
        assert_eq!(
            site_id("http://jiraportal.bse.com.uy/"),
            "jiraportal-bse-com-uy"
        );
        assert_eq!(site_id("https://Jira.Corp:8080"), "jira-corp-8080");
        assert_eq!(
            kind_from_host("https://acme.atlassian.net"),
            JiraKind::Cloud
        );
        assert_eq!(
            kind_from_host("http://jiraportal.bse.com.uy"),
            JiraKind::Server
        );
        assert_eq!(api_version(JiraKind::Cloud), "3");
        assert_eq!(search_path(JiraKind::Server), "/rest/api/2/search");
    }

    #[test]
    fn parse_sites_migrates_the_single_site_file_and_reads_the_new_one() {
        let old = parse_sites(r#"{"siteUrl":"https://acme.atlassian.net","email":"me@acme.com"}"#);
        assert_eq!(old.active.as_deref(), Some("acme-atlassian-net"));
        assert_eq!(old.sites.len(), 1);
        assert_eq!(old.sites[0].kind, JiraKind::Cloud);
        assert_eq!(old.sites[0].auth, JiraAuth::ApiToken);
        assert_eq!(old.sites[0].email, "me@acme.com");

        let new = parse_sites(
            r#"{"sites":[{"id":"a","siteUrl":"https://a.atlassian.net","email":"x"},{"id":"b","siteUrl":"http://jira.corp","email":"jdoe","kind":"server","auth":"pat"}],"active":"b"}"#,
        );
        assert_eq!(new.sites.len(), 2);
        assert_eq!(new.active.as_deref(), Some("b"));
        assert_eq!(new.sites[1].kind, JiraKind::Server);
        assert_eq!(new.sites[1].auth, JiraAuth::Pat);
        // Round-trips through serde unchanged.
        let txt = serde_json::to_string(&new).unwrap();
        assert_eq!(parse_sites(&txt), new);
        assert_eq!(parse_sites("{}"), JiraSites::default());
        assert_eq!(parse_sites("garbage"), JiraSites::default());
    }

    #[test]
    fn error_detail_quotes_jira_and_stays_short() {
        assert_eq!(
            error_detail(
                r#"{"errorMessages":["Field 'sprint' does not exist or you do not have permission to view it."],"errors":{}}"#
            ),
            " — Field 'sprint' does not exist or you do not have permission to view it."
        );
        assert_eq!(
            error_detail(r#"{"errorMessages":[],"errors":{"jql":"bad"}}"#),
            " — jql: bad"
        );
        assert_eq!(
            error_detail(r#"{"message":"XSRF check failed"}"#),
            " — XSRF check failed"
        );
        assert_eq!(error_detail("<html><body>502</body></html>"), "");
        assert_eq!(error_detail(""), "");
        let long = format!(r#"{{"errorMessages":["{}"]}}"#, "x".repeat(500));
        let d = error_detail(&long);
        assert!(d.ends_with('…') && d.chars().count() < 320, "{}", d.len());
    }

    #[test]
    fn worklog_body_speaks_adf_to_cloud_and_text_to_server() {
        let c = worklog_body(JiraKind::Cloud, 5400, "2026-10-26", Some(" fixed it "));
        assert_eq!(c["timeSpentSeconds"], 5400);
        assert_eq!(c["started"], "2026-10-26T12:00:00.000+0000");
        assert_eq!(c["comment"]["type"], "doc");
        assert_eq!(c["comment"]["content"][0]["content"][0]["text"], "fixed it");
        let s = worklog_body(JiraKind::Server, 60, "2026-10-26", Some("fixed it"));
        assert_eq!(s["comment"], "fixed it");
        assert!(worklog_body(JiraKind::Server, 60, "2026-10-26", Some("  "))
            .get("comment")
            .is_none());
    }

    #[test]
    fn authed_sends_bearer_for_a_pat_and_basic_otherwise() {
        let site = |auth: JiraAuth| JiraSite {
            id: "x".into(),
            site_url: "http://jira.corp".into(),
            email: "jdoe".into(),
            kind: JiraKind::Server,
            auth,
        };
        let c = reqwest::Client::new();
        let pat = authed(
            c.get("http://jira.corp/rest/api/2/myself"),
            &site(JiraAuth::Pat),
            "tok",
        )
        .build()
        .unwrap();
        assert_eq!(pat.headers()["authorization"], "Bearer tok");
        let pw = authed(
            c.get("http://jira.corp/rest/api/2/myself"),
            &site(JiraAuth::Password),
            "s3cret",
        )
        .build()
        .unwrap();
        let v = pw.headers()["authorization"].to_str().unwrap().to_string();
        assert!(v.starts_with("Basic "), "{v}");
        let cloud = authed(
            c.get("https://a.atlassian.net/rest/api/3/myself"),
            &JiraSite {
                kind: JiraKind::Cloud,
                auth: JiraAuth::ApiToken,
                ..site(JiraAuth::ApiToken)
            },
            "tok",
        )
        .build()
        .unwrap();
        assert!(cloud.headers()["authorization"]
            .to_str()
            .unwrap()
            .starts_with("Basic "));
    }

    #[test]
    fn flatten_derives_browse_url_and_tolerates_missing_fields() {
        let raw = RawIssue {
            key: "ABC-12".into(),
            fields: RawFields {
                summary: Some("Fix the thing".into()),
                status: Some(RawStatus {
                    name: "In Progress".into(),
                    status_category: Some(RawStatusCategory {
                        key: "indeterminate".into(),
                    }),
                }),
                priority: None,
                duedate: None,
                issuetype: Some(RawNamed { name: "Bug".into() }),
                project: Some(RawNamed {
                    name: "Acme".into(),
                }),
                updated: None,
            },
        };
        let out = flatten("https://acme.atlassian.net", raw);
        assert_eq!(out.url, "https://acme.atlassian.net/browse/ABC-12");
        assert_eq!(out.status_category, "indeterminate");
        assert_eq!(out.priority, None);
        assert_eq!(out.issue_type, "Bug");
    }

    #[test]
    fn duration_parsing_accepts_human_forms() {
        for (input, want) in [
            ("1h 30m", 5400),
            ("1h30m", 5400),
            ("90m", 5400),
            ("2h", 7200),
            ("1.5h", 5400),
            ("45", 2700),
            (" 15m ", 900),
        ] {
            assert_eq!(parse_duration_to_seconds(input).unwrap(), want, "{input}");
        }
    }

    #[test]
    fn duration_parsing_rejects_garbage_zero_and_absurd() {
        for bad in ["", "abc", "0", "0m", "-1h", "1h what", "m", "inf", "1000h"] {
            assert!(
                parse_duration_to_seconds(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn duration_formatting_round_trips_for_display() {
        assert_eq!(format_duration(5400), "1h 30m");
        assert_eq!(format_duration(2700), "45m");
        assert_eq!(format_duration(7200), "2h");
    }

    #[test]
    fn issue_key_and_date_validation() {
        assert!(valid_issue_key("FIX-123"));
        assert!(valid_issue_key("A2B-9"));
        assert!(!valid_issue_key("FIX"));
        assert!(!valid_issue_key("-123"));
        assert!(!valid_issue_key("2FIX-1"));
        assert!(!valid_issue_key("FIX-12a"));
        assert!(!valid_issue_key("FIX-1; rm"));
        assert!(valid_date("2026-07-22"));
        assert!(!valid_date("22/07/2026"));
        assert!(!valid_date("2026-7-22"));
    }

    #[test]
    fn worked_time_estimation_clusters_bursts() {
        const M: i64 = 60_000;
        // Empty / junk-only → zero.
        assert_eq!(estimate_worked_seconds(vec![], 15 * M, M), 0);
        assert_eq!(estimate_worked_seconds(vec![0, -5], 15 * M, M), 0);
        // A lone event counts the minimum burst (1m).
        assert_eq!(estimate_worked_seconds(vec![1_000_000], 15 * M, M), 60);
        // Events within the gap merge into one burst spanning first→last.
        // (Base offset because ts == 0 is the "hook gave no timestamp"
        // sentinel and gets filtered as junk.)
        const B: i64 = 100 * M;
        let burst = vec![B, B + 5 * M, B + 10 * M];
        assert_eq!(estimate_worked_seconds(burst, 15 * M, M), 600);
        // A >gap silence splits bursts; each contributes its own span.
        let two = vec![B, B + 10 * M, B + 60 * M, B + 70 * M];
        assert_eq!(estimate_worked_seconds(two, 15 * M, M), 1200);
        // Unsorted input is fine.
        let unsorted = vec![B + 70 * M, B, B + 60 * M, B + 10 * M];
        assert_eq!(estimate_worked_seconds(unsorted, 15 * M, M), 1200);
    }

    #[test]
    fn jql_validation_guards_junk() {
        assert!(validate_jql("assignee = currentUser()").is_ok());
        assert!(validate_jql("  ").is_err());
        assert!(validate_jql(&"x".repeat(1001)).is_err());
        assert!(validate_jql("a = b\u{0007}").is_err());
    }

    #[test]
    fn daily_digest_groups_marks_and_filters() {
        const M: i64 = 60_000;
        const B: i64 = 1_000 * M;
        let events = vec![
            ("jira:FIX-1".to_string(), B),
            ("jira:FIX-1".to_string(), B + 10 * M),
            ("jira:FIX-2".to_string(), B + 5 * M),
            // Outside the day window.
            ("jira:FIX-1".to_string(), B - 100 * M),
            // Non-jira cases are ignored.
            ("term:t-1".to_string(), B + 2 * M),
        ];
        let marks = vec![WorklogMark {
            issue_key: "FIX-2".into(),
            date: "2026-07-29".into(),
            seconds: 900,
            logged_at_ms: 1,
        }];
        let digest = build_daily_digest(&events, B - 50 * M, B + 50 * M, "2026-07-29", &marks);
        assert_eq!(digest.len(), 2);
        // FIX-1: burst 0..10m = 600s, biggest first.
        assert_eq!(digest[0].issue_key, "FIX-1");
        assert_eq!(digest[0].seconds, 600);
        assert_eq!(digest[0].events, 2);
        assert!(!digest[0].logged);
        // FIX-2: lone event = 1min floor, already marked logged for that date.
        assert_eq!(digest[1].issue_key, "FIX-2");
        assert_eq!(digest[1].seconds, 60);
        assert!(digest[1].logged);
        assert_eq!(digest[1].logged_seconds, Some(900));
        // A different date: FIX-2 counts as unlogged again.
        let digest = build_daily_digest(&events, B - 50 * M, B + 50 * M, "2026-07-30", &marks);
        assert!(!digest[1].logged);
    }

    /// One test fn — it mutates the process-global XDG_DATA_HOME.
    #[test]
    fn worklog_marks_round_trip_and_replace() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("ac-marks-{}-{nanos}", std::process::id()));
        let prev = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", &data);
        let root = format!("/proj/marks-{nanos}");

        let run = || -> AppResult<()> {
            assert!(load_marks(&root).is_empty());
            add_mark(
                &root,
                WorklogMark {
                    issue_key: "FIX-1".into(),
                    date: "2026-07-29".into(),
                    seconds: 3600,
                    logged_at_ms: 1,
                },
            )?;
            add_mark(
                &root,
                WorklogMark {
                    issue_key: "FIX-1".into(),
                    date: "2026-07-29".into(),
                    seconds: 5400,
                    logged_at_ms: 2,
                },
            )?;
            let marks = load_marks(&root);
            assert_eq!(marks.len(), 1, "same (issue, date) replaces");
            assert_eq!(marks[0].seconds, 5400);
            Ok(())
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        match prev {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        std::fs::remove_dir_all(&data).ok();
        match result {
            Ok(inner) => inner.unwrap(),
            Err(p) => std::panic::resume_unwind(p),
        }
    }
}
