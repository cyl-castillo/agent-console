use std::sync::Mutex;

use crate::services::agent_session::AgentRegistry;
use crate::services::project_manager::Project;
use crate::services::terminal_runner::TerminalRegistry;

/// Global application state. One active session at a time in v0.
pub struct AppState {
    pub inner: Mutex<SessionState>,
    pub terminals: TerminalRegistry,
    pub agent: AgentRegistry,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(SessionState::default()),
            terminals: TerminalRegistry::new(),
            agent: AgentRegistry::new(),
        }
    }
}

#[derive(Default)]
pub struct SessionState {
    pub project: Option<Project>,
}
