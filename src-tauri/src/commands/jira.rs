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
pub async fn jira_connect(
    site_url: String,
    email: String,
    token: String,
) -> AppResult<String> {
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
