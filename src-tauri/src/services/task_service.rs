use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Plan,
    Build,
    Debug,
    Review,
}

impl AgentMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentMode::Plan   => "plan",
            AgentMode::Build  => "build",
            AgentMode::Debug  => "debug",
            AgentMode::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub project_root: String,
    pub prompt: String,
    pub mode: AgentMode,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub created_at_ms: u64,
    #[serde(default)]
    pub completed_at_ms: Option<u64>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub snapshot_commit_sha: Option<String>,
    #[serde(default)]
    pub files_read: Vec<String>,
    #[serde(default)]
    pub files_modified: Vec<String>,
    #[serde(default)]
    pub commands_executed: Vec<String>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

const MAX_HISTORY: usize = 200;

fn store_path() -> AppResult<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AppError::Other("no config dir".into()))?
        .join("agent-console");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("tasks.json"))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    tasks: Vec<Task>,
}

fn load_all() -> Vec<Task> {
    let Ok(path) = store_path() else { return Vec::new() };
    let Ok(txt) = fs::read_to_string(&path) else { return Vec::new() };
    serde_json::from_str::<Store>(&txt).unwrap_or_default().tasks
}

fn write_all(tasks: Vec<Task>) -> AppResult<()> {
    let path = store_path()?;
    let store = Store { tasks };
    let json = serde_json::to_string_pretty(&store)
        .map_err(|e| AppError::Other(format!("serialize: {e}")))?;
    fs::write(&path, json)?;
    Ok(())
}

/// Save (insert or update) a task. Tasks are deduped by id; newest first.
pub fn save(task: &Task) -> AppResult<()> {
    let mut tasks = load_all();
    tasks.retain(|t| t.id != task.id);
    tasks.insert(0, task.clone());
    if tasks.len() > MAX_HISTORY {
        tasks.truncate(MAX_HISTORY);
    }
    write_all(tasks)
}

pub fn list(project_root: Option<&str>) -> Vec<Task> {
    let tasks = load_all();
    match project_root {
        Some(root) => tasks.into_iter().filter(|t| t.project_root == root).collect(),
        None       => tasks,
    }
}

pub fn delete(id: &str) -> AppResult<()> {
    let mut tasks = load_all();
    tasks.retain(|t| t.id != id);
    write_all(tasks)
}
