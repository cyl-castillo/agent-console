use parking_lot::Mutex;

use crate::services::activity_service::ActivityService;
use crate::services::git_watcher::GitWatcher;
use crate::services::hooks_service::HooksRuntime;
use crate::services::paired_devices_service::PairedDevicesService;
use crate::services::pairing_service::PairingService;
use crate::services::project_manager::Project;
use crate::services::roundtable_service::RoundtableService;
use crate::services::scheduler_service::SchedulerService;
use crate::services::sessions_service::SessionsService;
use crate::services::terminal_runner::TerminalRegistry;
use crate::services::testigo_service::TestigoService;
use crate::services::tts_service::TtsService;
use crate::services::voice_service::VoiceService;

pub struct AppState {
    pub inner: Mutex<SessionState>,
    pub terminals: TerminalRegistry,
    pub hooks: HooksRuntime,
    pub sessions: SessionsService,
    pub notes: crate::services::notes_service::NotesService,
    pub trust: crate::services::trust_service::TrustService,
    pub activity: ActivityService,
    /// Testigo: the hash-chained intent-to-proof ledger (prompts, approvals,
    /// snapshots, turn boundaries) — evidence, unlike `activity` which is a
    /// trimmed learning substrate.
    pub testigo: TestigoService,
    /// F8: the desktop's pairing identity (Noise static key) and the devices
    /// a human approved — shared with the listener's per-connection tasks.
    pub pairing: std::sync::Arc<PairingService>,
    pub paired_devices: std::sync::Arc<PairedDevicesService>,
    /// The remote-decisions listener, when started. None = stopped.
    pub remote_server: Mutex<Option<crate::services::transport::ServerHandle>>,
    pub git_watcher: GitWatcher,
    pub roundtable: RoundtableService,
    pub scheduler: SchedulerService,
    pub voice: VoiceService,
    pub tts: TtsService,
    /// Connector: agent-facing delegation/question/review queue behind the
    /// `agent_console` MCP server each room turn is given.
    pub connector: crate::services::connector_service::ConnectorService,
    /// The console's own MCP server (D9): pending webview roundtrips.
    pub console_mcp: crate::services::console_mcp::ConsoleMcp,
    /// D11: the sponsor policy per project (objective, budget, risk threshold).
    pub sponsor: crate::services::sponsor_service::SponsorService,
    /// F5: transcript tails streaming the agent's words into the conversation.
    pub transcripts: crate::services::transcript_service::TranscriptWatchers,
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
            trust: crate::services::trust_service::TrustService::new(),
            activity: ActivityService::new(),
            testigo: TestigoService::new(),
            pairing: std::sync::Arc::new(
                PairingService::load_keyring().expect("failed to load the device pairing identity"),
            ),
            paired_devices: std::sync::Arc::new(PairedDevicesService::new()),
            remote_server: Mutex::new(None),
            git_watcher: GitWatcher::new(),
            roundtable: RoundtableService::new(),
            scheduler: SchedulerService::new(),
            voice: VoiceService::default(),
            tts: TtsService::default(),
            connector: crate::services::connector_service::ConnectorService::new(),
            console_mcp: crate::services::console_mcp::ConsoleMcp::new(),
            sponsor: crate::services::sponsor_service::SponsorService::new(),
            transcripts: crate::services::transcript_service::TranscriptWatchers::new(),
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
