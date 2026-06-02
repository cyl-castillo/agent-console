// Typed wrappers around `invoke()`. One place to map Rust commands -> TS.

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  AdvisorAnalysisResult,
  BranchInfo,
  ContextFileStat, ContextStatus,
  FeedbackContext, FeedbackInput,
  FileContent, FileNode, GitCommitInfo, GitStatus, HooksStatus,
  InstalledPlugin, AvailableSnapshot, McpServer, McpAddInput,
  MemoryEntry, PermissionsSnapshot,
  PersistedSession, Project, RecentProject,
  RoundtableConfig,
  SessionUsage, Skill, StoredRule, VaultEntryView,
  WorkspaceContext,
} from "../types/domain";

export const ipc = {
  openProject: (path: string) => invoke<Project>("open_project", { path }),
  readTree: (path: string, depth = 3) =>
    invoke<FileNode>("read_tree", { path, depth }),
  currentProject: () => invoke<Project | null>("current_project"),
  readFileText: (path: string) => invoke<FileContent>("read_file_text", { path }),
  workspaceContext: () => invoke<WorkspaceContext>("workspace_context"),

  termSpawn: (cwd: string, termKey?: string) =>
    invoke<string>("term_spawn", { cwd, termKey: termKey ?? null }),
  termWrite: (id: string, data: string) =>
    invoke<void>("term_write", { id, data }),
  termResize: (id: string, cols: number, rows: number) =>
    invoke<void>("term_resize", { id, cols, rows }),
  termKill: (id: string) => invoke<void>("term_kill", { id }),

  gitStatus: () => invoke<GitStatus>("git_status"),
  gitDiffFile: (file: string) => invoke<string>("git_diff_file", { file }),
  gitRevertFile: (file: string) => invoke<void>("git_revert_file", { file }),
  gitStageFile: (file: string) => invoke<void>("git_stage_file", { file }),
  gitUnstageFile: (file: string) => invoke<void>("git_unstage_file", { file }),
  gitCommit: (message: string) => invoke<string>("git_commit", { message }),
  gitAmendCommit: (message: string) => invoke<string>("git_amend_commit", { message }),
  gitRecentMessages: (limit = 10) => invoke<string[]>("git_recent_messages", { limit }),
  gitHeadMessage: () => invoke<string>("git_head_message"),
  gitFileLog: (file: string, limit = 5) =>
    invoke<GitCommitInfo[]>("git_file_log", { file, limit }),
  gitBranches: () => invoke<BranchInfo[]>("git_branches"),
  gitCheckoutBranch: (name: string) => invoke<void>("git_checkout_branch", { name }),

  paletteIndexFiles: (limit?: number) =>
    invoke<string[]>("palette_index_files", { limit: limit ?? null }),

  snapshotRestore: (commitSha: string) =>
    invoke<void>("snapshot_restore", { commitSha }),
  snapshotDelete: (id: string) => invoke<void>("snapshot_delete", { id }),

  projectsRecent: () => invoke<RecentProject[]>("projects_recent"),
  projectsLast: () => invoke<RecentProject | null>("projects_last"),
  projectsForget: (path: string) => invoke<void>("projects_forget", { path }),
  projectsRemember: (path: string) => invoke<void>("projects_remember", { path }),

  skillList: () => invoke<Skill[]>("skill_list"),
  skillRead: (path: string) => invoke<string>("skill_read", { path }),

  hooksStatus: () => invoke<HooksStatus>("hooks_status"),
  hooksInstall: () => invoke<HooksStatus>("hooks_install"),
  hooksUninstall: () => invoke<HooksStatus>("hooks_uninstall"),

  approvalRespond: (id: string, decision: "allow" | "deny" | "ask", reason?: string) =>
    invoke<void>("approval_respond", { id, decision, reason: reason ?? null }),

  permissionsSnapshot: () => invoke<PermissionsSnapshot>("permissions_snapshot"),
  permissionsAdd: (
    scope: "project" | "global",
    effect: "allow" | "deny" | "ask",
    raw: string,
  ) => invoke<StoredRule>("permissions_add", { scope, effect, raw }),
  permissionsRemove: (
    scope: "project" | "global",
    effect: "allow" | "deny" | "ask",
    raw: string,
  ) => invoke<void>("permissions_remove", { scope, effect, raw }),

  sessionsList: (projectRoot: string) =>
    invoke<PersistedSession[]>("sessions_list", { projectRoot }),
  sessionsSave: (projectRoot: string, sessions: PersistedSession[]) =>
    invoke<void>("sessions_save", { projectRoot, sessions }),
  sessionUsage: (sessionId: string, projectRoot: string) =>
    invoke<SessionUsage | null>("session_usage", { sessionId, projectRoot }),

  contextStatus: () => invoke<ContextStatus>("context_status"),
  contextReadMd: (scope: "project" | "global") =>
    invoke<string>("context_read_md", { scope }),
  contextWriteMd: (scope: "project" | "global", content: string, expectedMtimeMs: number | null) =>
    invoke<ContextFileStat>("context_write_md", { scope, content, expectedMtimeMs }),
  contextOpenMdExternally: (scope: "project" | "global") =>
    invoke<void>("context_open_md_externally", { scope }),
  contextGenerateStarter: () => invoke<string>("context_generate_starter"),
  memoryList: () => invoke<MemoryEntry[]>("memory_list"),
  memoryRead: (name: string) => invoke<string>("memory_read", { name }),
  memoryDelete: (name: string) => invoke<void>("memory_delete", { name }),

  vaultList: () => invoke<VaultEntryView[]>("vault_list"),
  vaultUpsert: (params: {
    scope: "project" | "global";
    key: string;
    description: string;
    secret: boolean;
    value: string | null;
  }) => invoke<VaultEntryView>("vault_upsert", params),
  vaultDelete: (scope: "project" | "global", key: string) =>
    invoke<void>("vault_delete", { scope, key }),
  vaultGetValue: (scope: "project" | "global", key: string) =>
    invoke<string>("vault_get_value", { scope, key }),

  pluginsListInstalled: () => invoke<InstalledPlugin[]>("plugins_list_installed"),
  pluginsListAvailable: () => invoke<AvailableSnapshot>("plugins_list_available"),
  pluginsInstall: (installId: string, scope?: "user" | "project" | "local") =>
    invoke<string>("plugins_install", { installId, scope: scope ?? null }),

  mcpList: () => invoke<McpServer[]>("mcp_list"),
  mcpAdd: (input: McpAddInput) => invoke<string>("mcp_add", { input }),
  mcpRemove: (name: string, scope: string) =>
    invoke<string>("mcp_remove", { name, scope }),

  feedbackDevEnabled: () => invoke<boolean>("feedback_dev_enabled"),
  feedbackContext: () => invoke<FeedbackContext>("feedback_context"),
  feedbackSubmit: (input: FeedbackInput) =>
    invoke<string>("feedback_submit", { input }),

  advisorAnalyze: () => invoke<AdvisorAnalysisResult>("advisor_analyze"),
  advisorCreateSkill: (
    scope: "project" | "user",
    name: string,
    skillMdContent: string,
  ) =>
    invoke<string>("advisor_create_skill", { scope, name, skillMdContent }),

  roundtableStart: (config: RoundtableConfig) =>
    invoke<string>("roundtable_start", { config }),
  roundtablePause: (id: string) => invoke<void>("roundtable_pause", { id }),
  roundtableResume: (id: string) => invoke<void>("roundtable_resume", { id }),
  roundtableInject: (id: string, message: string) =>
    invoke<void>("roundtable_inject", { id, message }),
  roundtableStop: (id: string) => invoke<void>("roundtable_stop", { id }),
  roundtableSideDiff: (id: string, side: string) =>
    invoke<string>("roundtable_side_diff", { id, side }),
  roundtableApply: (id: string, side: string) =>
    invoke<string | null>("roundtable_apply", { id, side }),
  roundtableDiscard: (id: string) => invoke<void>("roundtable_discard", { id }),
};

export async function pickFolder(): Promise<string | null> {
  const result = await openDialog({ directory: true, multiple: false });
  if (Array.isArray(result)) return result[0] ?? null;
  return result ?? null;
}

export interface TermOutput { id: string; data: string }
export interface TermExit { id: string; code: number | null }
