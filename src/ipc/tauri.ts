// Typed wrappers around `invoke()`. One place to map Rust commands -> TS.

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  FileContent, FileNode, GitCommitInfo, GitStatus, HooksStatus, PermissionsSnapshot,
  PersistedSession, Project, RecentProject, Skill, StoredRule, WorkspaceContext,
} from "../types/domain";

export const ipc = {
  openProject: (path: string) => invoke<Project>("open_project", { path }),
  readTree: (path: string, depth = 3) =>
    invoke<FileNode>("read_tree", { path, depth }),
  currentProject: () => invoke<Project | null>("current_project"),
  readFileText: (path: string) => invoke<FileContent>("read_file_text", { path }),
  workspaceContext: () => invoke<WorkspaceContext>("workspace_context"),

  termSpawn: (cwd: string) => invoke<string>("term_spawn", { cwd }),
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
  gitFileLog: (file: string, limit = 5) =>
    invoke<GitCommitInfo[]>("git_file_log", { file, limit }),

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
};

export async function pickFolder(): Promise<string | null> {
  const result = await openDialog({ directory: true, multiple: false });
  if (Array.isArray(result)) return result[0] ?? null;
  return result ?? null;
}

export interface TermOutput { id: string; data: string }
export interface TermExit { id: string; code: number | null }
