use std::sync::Mutex;

use crate::services::activity_service::ActivityService;
use crate::services::git_watcher::GitWatcher;
use crate::services::hooks_service::HooksRuntime;
use crate::services::project_manager::Project;
use crate::services::roundtable_service::RoundtableService;
use crate::services::sessions_service::SessionsService;
use crate::services::terminal_runner::TerminalRegistry;
use crate::services::voice_service::VoiceService;

pub struct AppState {
    pub inner: Mutex<SessionState>,
    pub terminals: TerminalRegistry,
    pub hooks: HooksRuntime,
    pub sessions: SessionsService,
    pub activity: ActivityService,
    pub git_watcher: GitWatcher,
    pub roundtable: RoundtableService,
    pub voice: VoiceService,
}

impl AppState {
    pub fn new() -> Self {
        let hooks = HooksRuntime::new().expect("failed to initialize hooks runtime");
        Self {
            inner: Mutex::new(SessionState::default()),
            terminals: TerminalRegistry::new(),
            hooks,
            sessions: SessionsService::new(),
            activity: ActivityService::new(),
            git_watcher: GitWatcher::new(),
            roundtable: RoundtableService::new(),
            voice: VoiceService::default(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
pub struct SessionState {
    pub project: Option<Project>,
}
