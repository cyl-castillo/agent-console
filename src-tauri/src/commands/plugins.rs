use crate::error::{AppError, AppResult};
use crate::services::plugins_service::{self, AvailableSnapshot, InstalledPlugin};

#[tauri::command]
pub async fn plugins_list_installed() -> AppResult<Vec<InstalledPlugin>> {
    tokio::task::spawn_blocking(plugins_service::list_installed)
        .await
        .map_err(|e| AppError::Other(format!("plugins list task panicked: {e}")))
}

#[tauri::command]
pub async fn plugins_list_available() -> AppResult<AvailableSnapshot> {
    tokio::task::spawn_blocking(plugins_service::list_available)
        .await
        .map_err(|e| AppError::Other(format!("plugins available task panicked: {e}")))
}

#[tauri::command]
pub async fn plugins_install(install_id: String, scope: Option<String>) -> AppResult<String> {
    let scope = scope.unwrap_or_else(|| "user".to_string());
    tokio::task::spawn_blocking(move || plugins_service::install_plugin(&install_id, &scope))
        .await
        .map_err(|e| AppError::Other(format!("plugin install task panicked: {e}")))?
}
