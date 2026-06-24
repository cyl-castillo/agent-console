// Domain types shared with the Rust backend.
// Keep field names in sync with `serde(rename_all = "camelCase")` on Rust structs.

export interface Project {
  root: string;
  name: string;
  language: string | null;
  framework: string | null;
}

export interface FileNode {
  name: string;
  path: string;
  isDir: boolean;
  children?: FileNode[];
}

export interface GitFileChange {
  path: string;
  code: string;
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
}

export interface GitStatus {
  isRepo: boolean;
  branch: string | null;
  changes: GitFileChange[];
}

export interface BranchInfo {
  name: string;
  current: boolean;
  upstream: string | null;
  ahead: number;
  behind: number;
  lastCommitMs: number;
  lastSubject: string;
}

export interface GitCommitInfo {
  sha: string;
  shortSha: string;
  subject: string;
  author: string;
  dateMs: number;
}

export interface Snapshot {
  id: string;
  commitSha: string;
}

export interface RecentProject {
  path: string;
  name: string;
  lastOpenedMs: number;
}

export interface FileContent {
  content: string;
  isBinary: boolean;
  sizeBytes: number;
  truncated: boolean;
}

export interface WorkspaceContext {
  root: string;
  language: string | null;
  framework: string | null;
  fileCount: number;
  packageScripts: string[];
  entryPoints: string[];
  readmePreview: string | null;
}

/// A discovered Claude Code unit: skill, slash-command, or agent definition.
export interface Skill {
  name: string;
  kind: "skill" | "command" | "agent";
  source: "project" | "user";
  path: string;
  description: string | null;
  allowedTools: string[];
}

export interface PersistedSession {
  id: string;
  name: string;
  cwd: string;
  createdAtMs: number;
  scrollback: string;
  /// Which agent this session launches ("claude" | "codex"). Undefined = Claude.
  agent?: string;
  claudeSessionId?: string;
  nameSuggested?: boolean;
  /// Model alias / tuning value last chosen for this session, encoded by the
  /// agent profile on launch. Undefined = account/config default.
  model?: string;
}

export interface HooksStatus {
  sessionDir: string;
  scriptPath: string;
  pretooluseScriptPath: string;
  installed: boolean;
  pretooluseInstalled: boolean;
  settingsPath: string;
}

/// A PreToolUse approval request emitted by the bridge hook.
export interface ApprovalRequest {
  id: string;
  ts: number;
  sessionDir: string;
  cwd: string;
  tool: string;
  input: Record<string, unknown>;
}

export interface StoredRule {
  scope: "project" | "global";
  effect: "allow" | "deny" | "ask";
  raw: string;
  source: "agent-console" | "external";
  createdAtMs: number | null;
  settingsPath: string;
}

export interface PermissionsSnapshot {
  rules: StoredRule[];
  projectSettingsPath: string | null;
  globalSettingsPath: string;
}

/// Stat record returned by the Context service.
export interface ContextFileStat {
  path: string;
  exists: boolean;
  sizeBytes: number;
  modifiedMs: number;
}

export interface ContextDirStat {
  path: string;
  exists: boolean;
  entryCount: number;
}

export interface ContextStatus {
  projectClaudeMd: ContextFileStat | null;
  globalClaudeMd: ContextFileStat;
  memoryDir: ContextDirStat;
}

/// One entry inside the project's memory directory.
export interface MemoryEntry {
  name: string;
  kind: string | null;
  description: string | null;
  sizeBytes: number;
  modifiedMs: number;
  isIndex: boolean;
}

/// A Vault entry as exposed to the UI — never contains the value, only
/// metadata. Use `vaultGetValue` to fetch one on demand (reveal action).
export interface VaultEntryView {
  key: string;
  scope: "project" | "global";
  description: string;
  secret: boolean;
  hasValue: boolean;
  createdAtMs: number;
  updatedAtMs: number;
}

/// A skill recommendation returned by the Advisor analysis.
export interface AdvisorRecommendation {
  name: string;
  description: string;
  whyItFits: string;
  scope: "project" | "user";
  skillMdContent: string;
}

export interface AdvisorAnalysisResult {
  recommendations: AdvisorRecommendation[];
  rawExcerpt: string;
}

/// One improvement proposed by "learning mode" from observed daily activity.
/// kind: "skill" | "memory" | "friction".
export interface LearningSuggestion {
  kind: "skill" | "memory" | "friction";
  title: string;
  rationale: string;
  evidence: string[];
  skillName?: string;
  skillMdContent?: string;
  memoryName?: string;
  memoryContent?: string;
}

export interface ReflectionResult {
  suggestions: LearningSuggestion[];
  eventsAnalyzed: number;
  rawExcerpt: string;
}

/// One curation action proposed over the *existing* corpus (mirrors the Rust
/// CurationSuggestion). action: "merge" | "refactor" | "archive" | "rerank".
export interface CurationSuggestion {
  action: "merge" | "refactor" | "archive" | "rerank";
  targetKind: "skill" | "memory";
  targets: string[];
  title: string;
  rationale: string;
  evidence: string[];
  newName?: string;
  newContent?: string;
}

export interface CurationResult {
  suggestions: CurationSuggestion[];
  skillsAnalyzed: number;
  memoriesAnalyzed: number;
  rawExcerpt: string;
}

/// What makes a scheduled job fire (mirrors the Rust `Trigger` enum; tagged by
/// `type`). Daily/weekly `hour`/`minute` are UTC — the panel converts to/from
/// the user's local zone for display and editing.
export type Trigger =
  | { type: "interval"; everyMs: number }
  | { type: "daily"; hour: number; minute: number }
  | { type: "weekly"; weekday: number; hour: number; minute: number }
  | { type: "event"; name: string };

/// A gate on a pipeline step, evaluated against the previous executed step's
/// (status, output) (mirrors the Rust `StepCondition`).
export type StepCondition =
  | { type: "contains"; text: string }
  | { type: "prevFailed" }
  | { type: "prevOk" };

/// One pipeline step: an action plus an optional condition (mirrors the Rust
/// `PipelineStep`). No condition = run only if the prior step succeeded.
export interface PipelineStep {
  action: Action;
  when?: StepCondition;
}

/// What a scheduled job does when it fires (mirrors the Rust `Action` enum).
/// Every leaf runs through plan-mode `claude` (suggest-only).
export type Action =
  | { type: "skill"; name: string; args?: string }
  | { type: "prompt"; text: string }
  | { type: "pipeline"; steps: PipelineStep[] };

/// What to do with a firing missed because the app was closed.
export type OnMissed = "catchup" | "skip";

/// One scheduled job (mirrors the Rust `Job`).
export interface Job {
  id: string;
  name: string;
  enabled: boolean;
  trigger: Trigger;
  action: Action;
  onMissed: OnMissed;
  cooldownMs: number;
  createdAtMs: number;
  lastRunMs?: number;
  nextDueMs?: number;
}

/// One recorded execution of a job (mirrors the Rust `RunRecord`).
export interface RunRecord {
  jobId: string;
  jobName: string;
  startedMs: number;
  finishedMs: number;
  /// "ok" | "error" | "missed"
  status: string;
  summary: string;
  outputExcerpt: string;
}

/// A persisted activity-ledger record (mirrors the Rust ActivityEvent).
export interface ActivityEvent {
  ts: number;
  kind: string;
  prompt?: string;
  skill?: string;
  termId?: string;
  sessionId?: string;
  snapshotSha?: string;
}

/// A user_prompt event observed by the hook watcher.
export interface HookUserPromptEvent {
  type: "user_prompt";
  ts: number;
  prompt: string;
  skill?: string;
  sessionId?: string;
  /// Frontend terminal-session id the prompt came from (from AGENT_CONSOLE_TERM_ID).
  /// When present, the claude session id is bound to exactly this terminal.
  termId?: string;
}

export type FeedbackCategory = "bug" | "feature" | "ux" | "other";
export type FeedbackSeverity = "low" | "medium" | "high";

export interface FeedbackInput {
  title: string;
  description: string;
  category: FeedbackCategory;
  severity: FeedbackSeverity;
}

export interface FeedbackContext {
  appVersion: string;
  os: string;
  projectName: string | null;
  branch: string | null;
}

export interface InstalledPlugin {
  /// Full id, e.g. "rust-analyzer-lsp@claude-plugins-official".
  id: string;
  name: string;
  marketplace: string | null;
  version: string | null;
  scope: string | null;
  enabled: boolean;
  path: string | null;
}

export interface MarketplacePlugin {
  /// Id passed to `claude plugin install`, e.g. "name@marketplace".
  installId: string;
  name: string;
  marketplace: string;
  description: string;
  author: string | null;
  category: string | null;
  homepage: string | null;
}

export interface AvailableSnapshot {
  /// Names of configured marketplaces (empty => none added yet).
  marketplaces: string[];
  plugins: MarketplacePlugin[];
}

export interface McpServer {
  name: string;
  scope: string | null;       // local | user | project
  transport: string | null;   // stdio | http | sse
  command: string | null;
  args: string | null;
  url: string | null;
  env: string[];
  status: string;             // connected | failed | pending | unknown
  connected: boolean;
}

export interface McpAddInput {
  name: string;
  transport: "stdio" | "http" | "sse";
  scope: "local" | "user" | "project";
  commandOrUrl: string;
  env: string[];
  headers: string[];
}

/// Aggregated token usage for a Claude session, read from its transcript.
/// `contextTokens` is the latest turn's input+cache footprint (how full the
/// model context is); the totals are cumulative across the session.
export interface SessionUsage {
  contextTokens: number;
  inputTotal: number;
  outputTotal: number;
  cacheReadTotal: number;
  cacheCreationTotal: number;
  contextWindow: number;
}

// ----- Agent Room: N agents + the human converse about a problem (read-only) -----

export interface RoundtableParticipant {
  /// Stable key ("p1", "p2", …).
  id: string;
  /// Display name on this participant's messages.
  name: string;
  /// Which CLI backs this participant. Omitted = "claude" (backend default).
  engine?: "claude" | "codex";
  /// Claude: model alias ("opus"|"sonnet"). Codex: reasoning effort
  /// ("low"|"medium"|"high").
  model: string;
  /// Optional role/lens framing ("the skeptic", "the implementer", …).
  role: string;
}

export interface RoundtableConfig {
  /// The problem the room is working on.
  problem: string;
  /// Two or more agents; round-robin order is list order.
  participants: RoundtableParticipant[];
  /// Total AI turns across the conversation. Hard stop.
  maxTurns: number;
  /// Cumulative token ceiling across all agents. 0 = unlimited.
  tokenBudget: number;
  /// Working room: agents may edit the code in an isolated worktree
  /// (AcceptEdits), each turn auto-committed on a room/<id> branch for the human
  /// to review and merge. Off = conversation-only, read-only.
  allowEdits: boolean;
}

/// One message (agent turn or human injection), emitted over `roundtable://turn`.
export interface RoundtableTurn {
  id: string;
  authorId: string;
  authorName: string;
  /// null for the human.
  engine: "claude" | "codex" | null;
  model: string;
  text: string;
  turn: number;
  isHuman: boolean;
  totalTokens: number;
  costUsd: number;
}

/// A live activity line within a turn, emitted over `roundtable://activity`
/// as the agent works (so the feed streams what it's doing in real time).
export interface RoundtableActivity {
  id: string;
  authorId: string;
  turn: number;
  /// "thinking" | "tool" | "text"
  kind: string;
  /// For "tool": the tool name. Empty otherwise.
  label: string;
  /// For "tool": short arg summary. For "thinking"/"text": the content.
  text: string;
}

/// Lifecycle transition, emitted over `roundtable://status`.
export interface RoundtableStatus {
  id: string;
  /// "running" | "paused" | "done" | "stopped" | "error"
  status: string;
  turn: number;
  totalTokens: number;
  message: string | null;
}

// ----- Persisted rooms: a finished/in-progress room recoverable as history -----

/// One stored message in a room's transcript (agent turn or human injection).
export interface PersistedMessage {
  authorId: string;
  authorName: string;
  /// null for the human.
  engine: "claude" | "codex" | null;
  model: string;
  text: string;
  turn: number;
}

/// Full saved state of a room — fetched only when one is opened, for read-only
/// re-hydration of the panel. Never auto-resumes the underlying engines.
export interface PersistedRoom {
  version: number;
  id: string;
  problem: string;
  participants: RoundtableParticipant[];
  transcript: PersistedMessage[];
  /// Per-participant opaque engine resume references (not used until Fase B).
  resume: Record<string, string>;
  lastSeen: Record<string, number>;
  totalTokens: number;
  updatedAtMs: number;
}

/// Lightweight sidebar entry for a saved room (no transcript).
export interface RoomSummary {
  id: string;
  problem: string;
  participantNames: string[];
  messageCount: number;
  lastTurn: number;
  totalTokens: number;
  updatedAtMs: number;
}

/// Local voice input (push-to-talk → Whisper → composer).
export interface VoiceStatus {
  enabled: boolean;
  capturing: boolean;
  modelPresent: boolean;
  modelFile: string;
  language: string;
}

/// Payload of the `voice://model-progress` event during model download.
export interface VoiceModelProgress {
  downloaded: number;
  total: number | null;
}
