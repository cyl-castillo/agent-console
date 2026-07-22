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
pub async fn jira_list_issues() -> AppResult<Vec<JiraIssue>> {
    jira_service::list_assigned().await
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
