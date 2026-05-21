// Typed wrappers around `invoke()`. One place to map Rust commands -> TS.

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  FileContent, FileNode, GitStatus, Project, RecentProject, Snapshot,
} from "../types/domain";

export const ipc = {
  openProject: (path: string) => invoke<Project>("open_project", { path }),
  readTree: (path: string, depth = 3) =>
    invoke<FileNode>("read_tree", { path, depth }),
  currentProject: () => invoke<Project | null>("current_project"),
  readFileText: (path: string) => invoke<FileContent>("read_file_text", { path }),

  termSpawn: (cwd: string) => invoke<string>("term_spawn", { cwd }),
  termWrite: (id: string, data: string) =>
    invoke<void>("term_write", { id, data }),
  termResize: (id: string, cols: number, rows: number) =>
    invoke<void>("term_resize", { id, cols, rows }),
  termKill: (id: string) => invoke<void>("term_kill", { id }),

  gitStatus: () => invoke<GitStatus>("git_status"),
  gitDiffFile: (file: string) => invoke<string>("git_diff_file", { file }),
  gitRevertFile: (file: string) => invoke<void>("git_revert_file", { file }),

  chatSend: (text: string) => invoke<Snapshot | null>("chat_send", { text }),
  chatReset: () => invoke<void>("chat_reset"),

  snapshotRestore: (commitSha: string) =>
    invoke<void>("snapshot_restore", { commitSha }),
  snapshotDelete: (id: string) => invoke<void>("snapshot_delete", { id }),

  permRespond: (id: string, allow: boolean, reason: string | null = null) =>
    invoke<void>("perm_respond", { id, allow, reason }),
  permSetApproveAll: (enabled: boolean) =>
    invoke<void>("perm_set_approve_all", { enabled }),

  projectsRecent: () => invoke<RecentProject[]>("projects_recent"),
  projectsLast: () => invoke<RecentProject | null>("projects_last"),
  projectsForget: (path: string) => invoke<void>("projects_forget", { path }),
  projectsRemember: (path: string) => invoke<void>("projects_remember", { path }),
};

export async function pickFolder(): Promise<string | null> {
  const result = await openDialog({ directory: true, multiple: false });
  if (Array.isArray(result)) return result[0] ?? null;
  return result ?? null;
}

export interface TermOutput { id: string; data: string }
export interface TermExit { id: string; code: number | null }

export interface ChatAssistantText { text: string }
export interface ChatToolUse { id: string; name: string; input: unknown }
export interface ChatToolResult { toolUseId: string; ok: boolean; summary: string }
export interface ChatThinking { text: string }
export interface ChatDone { cost: number | null; error: string | null }
