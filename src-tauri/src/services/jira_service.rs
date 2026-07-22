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

const DEFAULT_JQL: &str = "assignee = currentUser() AND statusCategory != Done ORDER BY duedate ASC, priority DESC, updated DESC";

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
    let (cfg, token) = creds()?;
    let jql = match jql {
        Some(j) => validate_jql(j)?,
        None => DEFAULT_JQL,
    };
    let body = serde_json::json!({
        "jql": jql,
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

    let mut body = serde_json::json!({
        "timeSpentSeconds": secs,
        "started": format!("{started_date}T12:00:00.000+0000"),
    });
    if let Some(text) = comment.map(str::trim).filter(|t| !t.is_empty()) {
        body["comment"] = serde_json::json!({
            "type": "doc",
            "version": 1,
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }],
        });
    }

    let (cfg, token) = creds()?;
    let resp = client()?
        .post(format!("{}/rest/api/3/issue/{key}/worklog", cfg.site_url))
        .basic_auth(&cfg.email, Some(&token))
        .header("Accept", "application/json")
        .json(&body)
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
        s => Err(AppError::Other(format!("Jira worklog returned {s}"))),
    }
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
}
