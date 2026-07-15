use parking_lot::Mutex;

use crate::services::activity_service::ActivityService;
use crate::services::git_watcher::GitWatcher;
use crate::services::hooks_service::HooksRuntime;
use crate::services::project_manager::Project;
use crate::services::roundtable_service::RoundtableService;
use crate::services::scheduler_service::SchedulerService;
use crate::services::sessions_service::SessionsService;
use crate::services::terminal_runner::TerminalRegistry;
use crate::services::testigo_service::TestigoService;
use crate::services::voice_service::VoiceService;

pub struct AppState {
    pub inner: Mutex<SessionState>,
    pub terminals: TerminalRegistry,
    pub hooks: HooksRuntime,
    pub sessions: SessionsService,
    pub notes: crate::services::notes_service::NotesService,
    pub activity: ActivityService,
    /// Testigo: the hash-chained intent-to-proof ledger (prompts, approvals,
    /// snapshots, turn boundaries) — evidence, unlike `activity` which is a
    /// trimmed learning substrate.
    pub testigo: TestigoService,
    pub git_watcher: GitWatcher,
    pub roundtable: RoundtableService,
    pub scheduler: SchedulerService,
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
            notes: crate::services::notes_service::NotesService::new(),
            activity: ActivityService::new(),
            testigo: TestigoService::new(),
            git_watcher: GitWatcher::new(),
            roundtable: RoundtableService::new(),
            scheduler: SchedulerService::new(),
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
    /// When the active terminal session runs in an isolated worktree, git and
    /// snapshot commands operate on this checkout instead of the project root.
    /// None = the project root. Reset on project open and validated on set
    /// (must be a registered worktree of the open project).
    pub active_repo: Option<std::path::PathBuf>,
}
