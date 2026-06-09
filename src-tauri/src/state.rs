use std::sync::Mutex;

use crate::services::activity_service::ActivityService;
use crate::services::git_watcher::GitWatcher;
use crate::services::hooks_service::HooksRuntime;
use crate::services::paired_devices_service::PairedDevicesService;
use crate::services::pairing_service::PairingService;
use crate::services::project_manager::Project;
use crate::services::roundtable_service::RoundtableService;
use crate::services::sessions_service::SessionsService;
use crate::services::terminal_runner::TerminalRegistry;

pub struct AppState {
    pub inner: Mutex<SessionState>,
    pub terminals: TerminalRegistry,
    pub hooks: HooksRuntime,
    pub sessions: SessionsService,
    pub activity: ActivityService,
    pub git_watcher: GitWatcher,
    pub roundtable: RoundtableService,
    /// Mobile voice companion: desktop pairing identity + orchestrator.
    pub pairing: PairingService,
    /// Trusted paired devices + pending-approval gate.
    pub paired_devices: PairedDevicesService,
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
            pairing: PairingService::load().expect("failed to load device pairing identity"),
            paired_devices: PairedDevicesService::new(),
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
