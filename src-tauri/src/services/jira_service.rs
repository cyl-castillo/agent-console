//! Jira Cloud integration: see the issues assigned to you and turn one into an
//! agent session. Read-mostly by design.
//!
//! - Credentials: the site URL + email live in a small global JSON config; the
//!   API token lives in the OS keychain (never in the JSON, never in a log).
//!   The token only ever leaves the machine as a Basic-auth header to the
//!   user's own Jira site.
//! - API: Jira Cloud REST v3. The legacy `/rest/api/3/search` was removed, so
//!   we use `POST /rest/api/3/search/jql`.

use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

const KEYRING_SERVICE: &str = "agent-console:jira";
const KEYRING_ACCOUNT: &str = "api-token";

/// The non-secret half of the connection, persisted as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraConfig {
    /// Cloud site base, e.g. `https://acme.atlassian.net`.
    pub site_url: String,
    pub email: String,
}

/// What the UI needs to decide "connect form" vs "issue list", without ever
/// shipping the token to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraStatus {
    pub configured: bool,
    pub site_url: String,
    pub email: String,
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

fn keyring_entry() -> AppResult<Entry> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| AppError::Other(format!("keyring open: {e}")))
}

pub fn load_config() -> Option<JiraConfig> {
    let path = config_path().ok()?;
    let txt = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&txt).ok()
}

fn get_token() -> Option<String> {
    keyring_entry().ok()?.get_password().ok()
}

/// Normalize/validate the site URL: must be https and have a host. Trailing
/// slash trimmed so we can concatenate `/rest/...` cleanly.
fn normalize_site(raw: &str) -> AppResult<String> {
    let s = raw.trim().trim_end_matches('/');
    if !s.starts_with("https://") || s.len() < "https://a.b".len() {
        return Err(AppError::InvalidArgument(
            "site URL must be https:// (e.g. https://yourteam.atlassian.net)".into(),
        ));
    }
    // No spaces or control chars — it becomes a request URL.
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AppError::InvalidArgument(
            "site URL contains invalid characters".into(),
        ));
    }
    Ok(s.to_string())
}

pub fn status() -> JiraStatus {
    let cfg = load_config().unwrap_or_default();
    let has_token = get_token().is_some();
    JiraStatus {
        configured: !cfg.site_url.is_empty() && !cfg.email.is_empty() && has_token,
        site_url: cfg.site_url,
        email: cfg.email,
    }
}

/// Persist site + email as JSON and the token in the keychain. The token is
/// required here so "save" and "connect" are one atomic action.
pub fn save(site_url: &str, email: &str, token: &str) -> AppResult<()> {
    let site = normalize_site(site_url)?;
    let email = email.trim();
    if email.is_empty() {
        return Err(AppError::InvalidArgument("email is required".into()));
    }
    if token.trim().is_empty() {
        return Err(AppError::InvalidArgument("API token is required".into()));
    }
    let cfg = JiraConfig {
        site_url: site,
        email: email.to_string(),
    };
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| AppError::Other(e.to_string()))?;
    std::fs::write(config_path()?, json)?;
    keyring_entry()?
        .set_password(token.trim())
        .map_err(|e| AppError::Other(format!("keyring set: {e}")))?;
    Ok(())
}

/// Forget the connection entirely: config file + keychain token.
pub fn disconnect() -> AppResult<()> {
    if let Ok(path) = config_path() {
        let _ = std::fs::remove_file(path);
    }
    if let Ok(entry) = keyring_entry() {
        let _ = entry.delete_credential();
    }
    Ok(())
}

fn client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("agent-console")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Other(format!("http client: {e}")))
}

fn creds() -> AppResult<(JiraConfig, String)> {
    let cfg = load_config()
        .filter(|c| !c.site_url.is_empty() && !c.email.is_empty())
        .ok_or_else(|| AppError::InvalidArgument("Jira is not configured".into()))?;
    let token = get_token()
        .ok_or_else(|| AppError::InvalidArgument("Jira token missing from keychain".into()))?;
    Ok((cfg, token))
}

/// Validate the stored credentials against `GET /myself`. Returns the account's
/// display name on success — used by the connect form as a confirmation.
pub async fn test_connection() -> AppResult<String> {
    let (cfg, token) = creds()?;
    let resp = client()?
        .get(format!("{}/rest/api/3/myself", cfg.site_url))
        .basic_auth(&cfg.email, Some(&token))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Jira request failed: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Other(
            "Jira rejected the credentials (401) — check the email and API token".into(),
        ));
    }
    if !resp.status().is_success() {
        return Err(AppError::Other(format!("Jira returned {}", resp.status())));
    }
    let me: MyselfResp = resp
        .json()
        .await
        .map_err(|e| AppError::Other(format!("bad Jira response: {e}")))?;
    Ok(me.display_name.unwrap_or_else(|| cfg.email.clone()))
}

/// Issues assigned to the current user that aren't Done, soonest due first.
pub async fn list_assigned() -> AppResult<Vec<JiraIssue>> {
    let (cfg, token) = creds()?;
    let body = serde_json::json!({
        "jql": "assignee = currentUser() AND statusCategory != Done ORDER BY duedate ASC, priority DESC, updated DESC",
        "maxResults": 50,
        "fields": ["summary", "status", "priority", "duedate", "issuetype", "project", "updated"],
    });
    let resp = client()?
        .post(format!("{}/rest/api/3/search/jql", cfg.site_url))
        .basic_auth(&cfg.email, Some(&token))
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Other(format!("Jira request failed: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Other(
            "Jira rejected the credentials (401)".into(),
        ));
    }
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "Jira search returned {}",
            resp.status()
        )));
    }
    let parsed: SearchResp = resp
        .json()
        .await
        .map_err(|e| AppError::Other(format!("bad Jira response: {e}")))?;
    Ok(parsed
        .issues
        .into_iter()
        .map(|i| flatten(&cfg.site_url, i))
        .collect())
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
    fn normalize_site_enforces_https_and_trims() {
        assert_eq!(
            normalize_site("https://acme.atlassian.net/").unwrap(),
            "https://acme.atlassian.net"
        );
        assert!(normalize_site("http://acme.atlassian.net").is_err());
        assert!(normalize_site("acme.atlassian.net").is_err());
        assert!(normalize_site("https://a b.net").is_err());
        assert!(normalize_site("").is_err());
    }

    #[test]
    fn save_rejects_blank_email_and_token() {
        assert!(save("https://x.atlassian.net", "  ", "tok").is_err());
        assert!(save("https://x.atlassian.net", "me@x.com", "  ").is_err());
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
}
