use crate::error::AppResult;
use crate::services::teams_service::{
    self, DeviceLoginStart, LoginPoll, TeamsChat, TeamsMessage, TeamsStatus,
};

#[tauri::command]
pub fn teams_status() -> TeamsStatus {
    teams_service::status()
}

/// Kick off the device code flow. Returns the user-facing code + URL; the
/// sensitive device_code stays in the backend.
#[tauri::command]
pub async fn teams_begin_login(client_id: String, tenant: String) -> AppResult<DeviceLoginStart> {
    teams_service::begin_login(&client_id, &tenant).await
}

/// One poll of the pending login; the frontend repeats this at the interval
/// `teams_begin_login` returned until it's Connected or Failed.
#[tauri::command]
pub async fn teams_poll_login() -> AppResult<LoginPoll> {
    teams_service::poll_login().await
}

/// Abandon a login the user walked away from (form closed).
#[tauri::command]
pub fn teams_cancel_login() {
    teams_service::cancel_login()
}

#[tauri::command]
pub fn teams_disconnect() -> AppResult<()> {
    teams_service::disconnect()
}

#[tauri::command]
pub async fn teams_list_chats() -> AppResult<Vec<TeamsChat>> {
    teams_service::list_chats().await
}

#[tauri::command]
pub async fn teams_list_messages(chat_id: String) -> AppResult<Vec<TeamsMessage>> {
    teams_service::list_messages(&chat_id).await
}
