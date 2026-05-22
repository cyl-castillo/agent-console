use std::sync::Mutex;

use crate::services::hooks_service::HooksRuntime;
use crate::services::project_manager::Project;
use crate::services::sessions_service::SessionsService;
use crate::services::terminal_runner::TerminalRegistry;

pub struct AppState {
    pub inner: Mutex<SessionState>,
    pub terminals: TerminalRegistry,
    pub hooks: HooksRuntime,
    pub sessions: SessionsService,
}

impl AppState {
    pub fn new() -> Self {
        let hooks = HooksRuntime::new()
            .expect("failed to initialize hooks runtime");
        Self {
            inner: Mutex::new(SessionState::default()),
            terminals: TerminalRegistry::new(),
            hooks,
            sessions: SessionsService::new(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self { Self::new() }
}

#[derive(Default)]
pub struct SessionState {
    pub project: Option<Project>,
}
