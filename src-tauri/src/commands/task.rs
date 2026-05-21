use crate::error::AppResult;
use crate::services::task_service::{self, Task};

#[tauri::command]
pub fn task_save(task: Task) -> AppResult<()> {
    task_service::save(&task)
}

#[tauri::command]
pub fn task_list(project_root: Option<String>) -> AppResult<Vec<Task>> {
    Ok(task_service::list(project_root.as_deref()))
}

#[tauri::command]
pub fn task_delete(id: String) -> AppResult<()> {
    task_service::delete(&id)
}
