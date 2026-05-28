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
  claudeSessionId?: string;
  nameSuggested?: boolean;
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
  name: string;
  slug: string;
  version: string | null;
  description: string | null;
  path: string;
}

export interface MarketplacePlugin {
  name: string;
  slug: string;
  description: string;
  author: string | null;
  repoUrl: string | null;
  tags: string[];
}

export interface MarketplaceSnapshot {
  source: string;
  fetchedAtMs: number;
  isFallback: boolean;
  plugins: MarketplacePlugin[];
}
