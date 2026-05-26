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

/// A user_prompt event observed by the hook watcher.
export interface HookUserPromptEvent {
  type: "user_prompt";
  ts: number;
  prompt: string;
  skill?: string;
  sessionId?: string;
}
