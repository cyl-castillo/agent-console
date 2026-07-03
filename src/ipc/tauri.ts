// Typed wrappers around `invoke()`. One place to map Rust commands -> TS.

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type {
  ActivityEvent,
  AdvisorAnalysisResult,
  BranchInfo,
  ContextFileStat, ContextStatus,
  CurationResult,
  ExportOptions, ExportResult,
  ImportDecisions, ImportManifest, ImportResult,
  FeedbackContext, FeedbackInput,
  FileContent, FileNode, GitCommitInfo, GitStatus, HooksStatus,
  InstalledPlugin, AvailableSnapshot, Job, McpServer, McpAddInput,
  MemoryEntry, PermissionsSnapshot, Preflight,
  PersistedRoom, PersistedSession, Project, RecentProject,
  RoomSummary, RoundtableConfig, RunRecord,
  ReflectionResult,
  SessionUsage, Skill, StoredRule, VaultEntryView,
  VoiceStatus,
  WorkspaceContext,
  WorktreeCreated, WorktreeRef, WorktreeSetupConfig, WorktreeStatusInfo,
  MergeOutcome,
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
  // Save pasted image bytes to a temp file; returns its absolute path.
  // Raw-body invoke: bytes travel as the request body, ext as a header.
  termSavePasteImage: (bytes: Uint8Array, ext: string) =>
    invoke<string>("term_save_paste_image", bytes, {
      headers: { "x-image-ext": ext },
    }),
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

  // Returns the "pre-restore" snapshot sha (a backup of the tree taken right
  // before the destructive restore), or null if the repo couldn't be snapshotted.
  snapshotRestore: (commitSha: string) =>
    invoke<string | null>("snapshot_restore", { commitSha }),
  snapshotDelete: (id: string) => invoke<void>("snapshot_delete", { id }),

  preflightCheck: () => invoke<Preflight>("preflight_check"),

  // Per-session isolated worktrees. Destructive ops (merge/discard) validate
  // on the Rust side that the path is a registered worktree of the repo.
  worktreeCreate: (name: string, base?: string) =>
    invoke<WorktreeCreated>("worktree_create", { name, base: base ?? null }),
  worktreeStatus: (wt: WorktreeRef) =>
    invoke<WorktreeStatusInfo>("worktree_status", {
      path: wt.path, branch: wt.branch, baseBranch: wt.baseBranch,
    }),
  worktreeMerge: (wt: WorktreeRef, deleteAfter: boolean) =>
    invoke<MergeOutcome>("worktree_merge", {
      path: wt.path, branch: wt.branch, baseBranch: wt.baseBranch, deleteAfter,
    }),
  worktreeDiscard: (wt: WorktreeRef, deleteBranch: boolean) =>
    invoke<void>("worktree_discard", {
      path: wt.path, branch: wt.branch, deleteBranch,
    }),
  worktreeSetupGet: () => invoke<WorktreeSetupConfig>("worktree_setup_get"),
  worktreeSetupSet: (config: WorktreeSetupConfig) =>
    invoke<void>("worktree_setup_set", { config }),
  // Aim git/snapshot commands + the change watcher at the active session's
  // checkout (null = back to the project root).
  setActiveRepo: (path: string | null) =>
    invoke<void>("set_active_repo", { path }),
  worktreePruneOrphans: (keep: string[]) =>
    invoke<string[]>("worktree_prune_orphans", { keep }),

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

  learningReflect: (limit?: number) =>
    invoke<ReflectionResult>("learning_reflect", { limit: limit ?? null }),
  activityList: (limit?: number) =>
    invoke<ActivityEvent[]>("activity_list", { limit: limit ?? null }),
  learningCreateSkill: (name: string, skillMdContent: string) =>
    invoke<string>("learning_create_skill", { name, skillMdContent }),
  learningSaveMemory: (name: string, content: string) =>
    invoke<string>("learning_save_memory", { name, content }),
  learningCurate: () => invoke<CurationResult>("learning_curate", {}),
  learningApplyRefactor: (
    targetKind: "skill" | "memory",
    name: string,
    newContent: string,
  ) =>
    invoke<string>("learning_apply_refactor", { targetKind, name, newContent }),
  learningApplyMerge: (
    targetKind: "skill" | "memory",
    targets: string[],
    newName: string,
    newContent: string,
  ) =>
    invoke<string>("learning_apply_merge", { targetKind, targets, newName, newContent }),
  learningApplyArchive: (targetKind: "skill" | "memory", name: string) =>
    invoke<string>("learning_apply_archive", { targetKind, name }),

  // --- scheduler (visual jobs on a clock; suggest-only via plan mode) ---
  schedulerList: () => invoke<Job[]>("scheduler_list"),
  schedulerCreate: (job: Job) => invoke<Job>("scheduler_create", { job }),
  schedulerUpdate: (job: Job) => invoke<Job>("scheduler_update", { job }),
  schedulerDelete: (id: string) => invoke<void>("scheduler_delete", { id }),
  schedulerSetEnabled: (id: string, enabled: boolean) =>
    invoke<Job>("scheduler_set_enabled", { id, enabled }),
  schedulerHistory: (limit?: number) =>
    invoke<RunRecord[]>("scheduler_history", { limit: limit ?? null }),
  schedulerFireEvent: (name: string) => invoke<void>("scheduler_fire_event", { name }),
  schedulerIsPaused: () => invoke<boolean>("scheduler_is_paused"),
  schedulerSetPaused: (paused: boolean) => invoke<void>("scheduler_set_paused", { paused }),
  schedulerRunNow: (id: string) => invoke<RunRecord>("scheduler_run_now", { id }),

  roundtableStart: (config: RoundtableConfig) =>
    invoke<string>("roundtable_start", { config }),
  roundtablePause: (id: string) => invoke<void>("roundtable_pause", { id }),
  roundtableResume: (id: string) => invoke<void>("roundtable_resume", { id }),
  roundtableInject: (id: string, message: string) =>
    invoke<void>("roundtable_inject", { id, message }),
  roundtableContinue: (id: string, extra: number) =>
    invoke<void>("roundtable_continue", { id, extra }),
  roundtableStop: (id: string) => invoke<void>("roundtable_stop", { id }),
  roundtableDiscard: (id: string) => invoke<void>("roundtable_discard", { id }),
  roundtableListRooms: () => invoke<RoomSummary[]>("roundtable_list_rooms"),
  roundtableGetRoom: (id: string) =>
    invoke<PersistedRoom | null>("roundtable_get_room", { id }),
  roundtableDeleteRoom: (id: string) =>
    invoke<void>("roundtable_delete_room", { id }),
  roundtableResumeRoom: (id: string) =>
    invoke<string>("roundtable_resume_room", { id }),

  voiceStatus: () => invoke<VoiceStatus>("voice_status"),
  voiceEnable: () => invoke<VoiceStatus>("voice_enable"),
  voiceDisable: () => invoke<VoiceStatus>("voice_disable"),
  voicePttStart: () => invoke<void>("voice_ptt_start"),
  voicePttStop: () => invoke<string>("voice_ptt_stop"),
  voiceSpeak: (text: string) => invoke<void>("voice_speak", { text }),
  voiceListen: (seconds: number) => invoke<string>("voice_listen", { seconds }),

  // Bundle the chosen blocks of work for `projectRoot` and write the archive to
  // `destPath` (a path from the save dialog). Returns counts for a confirmation.
  exportWork: (projectRoot: string, options: ExportOptions, destPath: string) =>
    invoke<ExportResult>("export_work", { projectRoot, options, destPath }),

  // Preview what importing `srcPath` into `projectRoot` would do (no mutation).
  importWorkPreview: (projectRoot: string, srcPath: string) =>
    invoke<ImportManifest>("import_work_preview", { projectRoot, srcPath }),

  // Apply `srcPath` into `projectRoot` with the user's per-block decisions.
  importWorkApply: (
    projectRoot: string,
    srcPath: string,
    decisions: ImportDecisions,
  ) => invoke<ImportResult>("import_work_apply", { projectRoot, srcPath, decisions }),
};

export async function pickFolder(): Promise<string | null> {
  const result = await openDialog({ directory: true, multiple: false });
  if (Array.isArray(result)) return result[0] ?? null;
  return result ?? null;
}

/// Save-file picker for an export archive. Defaults the name and `.acwork`
/// extension; returns the chosen path or null if cancelled.
export async function pickSaveFile(defaultName: string): Promise<string | null> {
  const result = await saveDialog({
    defaultPath: defaultName,
    filters: [{ name: "Agent Console workspace", extensions: ["acwork"] }],
  });
  return result ?? null;
}

/// Open-file picker for an import archive (`.acwork`). Returns the chosen path
/// or null if cancelled.
export async function pickOpenFile(): Promise<string | null> {
  const result = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: "Agent Console workspace", extensions: ["acwork"] }],
  });
  if (Array.isArray(result)) return result[0] ?? null;
  return result ?? null;
}

export interface TermOutput { id: string; data: string }
export interface TermExit { id: string; code: number | null }
