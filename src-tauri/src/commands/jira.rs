use crate::error::AppResult;
use crate::services::jira_service::{self, JiraIssue, JiraStatus};

#[tauri::command]
pub fn jira_status() -> JiraStatus {
    jira_service::status()
}

/// Save the connection AND validate it in one step: persist, then hit
/// `/myself`. If validation fails, roll the save back so a bad token doesn't
/// leave the app looking "configured". Returns the account display name.
#[tauri::command]
pub async fn jira_connect(site_url: String, email: String, token: String) -> AppResult<String> {
    jira_service::save(&site_url, &email, &token)?;
    match jira_service::test_connection().await {
        Ok(name) => Ok(name),
        Err(e) => {
            let _ = jira_service::disconnect();
            Err(e)
        }
    }
}

#[tauri::command]
pub fn jira_disconnect() -> AppResult<()> {
    jira_service::disconnect()
}

#[tauri::command]
pub async fn jira_list_issues(jql: Option<String>) -> AppResult<Vec<JiraIssue>> {
    jira_service::list_assigned(jql.as_deref()).await
}

/// Log time on an issue. `duration` is human ("1h 30m"), `started` is
/// YYYY-MM-DD. Returns the normalized label that was logged.
#[tauri::command]
pub async fn jira_log_work(
    issue_key: String,
    duration: String,
    started: String,
    comment: Option<String>,
) -> AppResult<String> {
    jira_service::log_work(&issue_key, &duration, &started, comment.as_deref()).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorklogSuggestion {
    pub seconds: u64,
    pub events: usize,
    pub first_ts: i64,
    pub last_ts: i64,
}

/// Suggest a worklog duration for an issue from the witnessed activity of its
/// `jira:<KEY>` case in the Testigo ledger, within [day_start_ms, day_end_ms)
/// (local-day bounds computed by the frontend). None = no meaningful activity.
#[tauri::command]
pub fn jira_worklog_suggestion(
    project_root: String,
    issue_key: String,
    day_start_ms: i64,
    day_end_ms: i64,
    state: tauri::State<'_, crate::state::AppState>,
) -> AppResult<Option<WorklogSuggestion>> {
    let case = format!("jira:{}", issue_key.trim().to_uppercase());
    let events = state.testigo.list(&project_root, Some(&case), None)?;
    let ts: Vec<i64> = events
        .iter()
        .map(|e| e.ts)
        .filter(|t| *t >= day_start_ms && *t < day_end_ms)
        .collect();
    if ts.is_empty() {
        return Ok(None);
    }
    let first_ts = *ts.iter().min().unwrap();
    let last_ts = *ts.iter().max().unwrap();
    let count = ts.len();
    let seconds = jira_service::estimate_worked_seconds(ts, 15 * 60 * 1000, 60 * 1000);
    if seconds < 60 {
        return Ok(None);
    }
    Ok(Some(WorklogSuggestion {
        seconds,
        events: count,
        first_ts,
        last_ts,
    }))
}

/// The "⏱ Today" digest: tickets with witnessed activity in the given local
/// day (bounds computed by the frontend), with estimated time and whether
/// that (ticket, day) was already logged.
#[tauri::command]
pub fn jira_daily_digest(
    project_root: String,
    day_start_ms: i64,
    day_end_ms: i64,
    date: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> AppResult<Vec<jira_service::DigestEntry>> {
    let events = state.testigo.list(&project_root, None, None)?;
    let pairs: Vec<(String, i64)> = events.into_iter().map(|e| (e.case_id, e.ts)).collect();
    let marks = jira_service::load_marks(&project_root);
    Ok(jira_service::build_daily_digest(
        &pairs,
        day_start_ms,
        day_end_ms,
        &date,
        &marks,
    ))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayLogEntry {
    pub issue_key: String,
    /// Human duration as (possibly) edited by the user ("1h 30m").
    pub duration: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayLogResult {
    pub issue_key: String,
    pub ok: bool,
    pub message: String,
}

/// Log a whole day's entries in one go. Per-entry results (one bad ticket
/// doesn't sink the batch); successes are marked so the digest shows them as
/// done and never double-logs.
#[tauri::command]
pub async fn jira_log_day(
    project_root: String,
    date: String,
    entries: Vec<DayLogEntry>,
) -> AppResult<Vec<DayLogResult>> {
    let mut results = Vec::with_capacity(entries.len());
    for e in entries {
        match jira_service::log_work(&e.issue_key, &e.duration, &date, None).await {
            Ok(label) => {
                let seconds = jira_service::parse_duration_to_seconds(&e.duration).unwrap_or(0);
                let _ = jira_service::add_mark(
                    &project_root,
                    jira_service::WorklogMark {
                        issue_key: e.issue_key.clone(),
                        date: date.clone(),
                        seconds,
                        logged_at_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0),
                    },
                );
                results.push(DayLogResult {
                    issue_key: e.issue_key,
                    ok: true,
                    message: label,
                });
            }
            Err(err) => results.push(DayLogResult {
                issue_key: e.issue_key,
                ok: false,
                message: err.to_string(),
            }),
        }
    }
    Ok(results)
}
