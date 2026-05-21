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

export type ToolStatus = "running" | "ok" | "error";

export interface Snapshot {
  id: string;
  commitSha: string;
}

export type ChatBlock =
  | { kind: "user"; id: string; content: string; snapshot?: Snapshot; restored?: boolean }
  | { kind: "text"; id: string; content: string }
  | { kind: "tool"; id: string; name: string; input: unknown; status: ToolStatus; summary?: string }
  | { kind: "thinking"; id: string; content: string };

export interface RecentProject {
  path: string;
  name: string;
  lastOpenedMs: number;
}

export interface PermissionRequest {
  id: string;
  toolName: string;
  toolInput: unknown;
}

export interface FileContent {
  content: string;
  isBinary: boolean;
  sizeBytes: number;
  truncated: boolean;
}
