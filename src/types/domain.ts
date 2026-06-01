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
