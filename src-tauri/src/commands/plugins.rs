use crate::error::AppResult;
use crate::services::plugins_service::{
    self, InstalledPlugin, MarketplaceSnapshot,
};

#[tauri::command]
pub fn plugins_list_installed() -> Vec<InstalledPlugin> {
    plugins_service::list_installed()
}

#[tauri::command]
pub async fn plugins_marketplace(force: Option<bool>) -> AppResult<MarketplaceSnapshot> {
    plugins_service::fetch_marketplace(force.unwrap_or(false)).await
}
