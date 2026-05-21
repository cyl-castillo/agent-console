import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  ipc,
  type ChatAssistantText,
  type ChatDone,
  type ChatThinking,
  type ChatToolResult,
  type ChatToolUse,
} from "../ipc/tauri";
import type { ChatBlock, PermissionRequest } from "../types/domain";
import { useChangesStore } from "./changesStore";
import { useTaskStore } from "./taskStore";
import { HELP_TEXT, parseCommand } from "../utils/parseCommand";

const MUTATING_TOOLS = new Set([
  "Edit", "Write", "MultiEdit", "NotebookEdit", "Bash",
]);
const READ_TOOLS = new Set(["Read", "Glob", "Grep", "LS"]);
const DANGEROUS_TOOLS = new Set([
  "Edit", "Write", "MultiEdit", "NotebookEdit", "Bash",
]);

interface ChatState {
  blocks: ChatBlock[];
  sending: boolean;
  inputDraft: string;
  totalCost: number;
  pendingPermissions: PermissionRequest[];
  approveAll: boolean;
  autoSwitchSignal: number;
  hasMutatedThisTurn: boolean;

  setDraft: (s: string) => void;
  send: (text: string) => Promise<void>;
  reset: () => Promise<void>;
  decidePermission: (allow: boolean) => Promise<void>;
  setApproveAll: (on: boolean) => Promise<void>;
  restoreSnapshot: (commitSha: string, userBlockId: string) => Promise<void>;

  _onText: (text: string) => void;
  _onToolUse: (e: ChatToolUse) => void;
  _onToolResult: (e: ChatToolResult) => void;
  _onThinking: (e: ChatThinking) => void;
  _onDone: (e: ChatDone) => void;
  _onPerm: (r: PermissionRequest) => void;
}

export const useChatStore = create<ChatState>((set, get) => ({
  blocks: [],
  sending: false,
  inputDraft: "",
  totalCost: 0,
  pendingPermissions: [],
  approveAll: false,
  autoSwitchSignal: 0,
  hasMutatedThisTurn: false,

  setDraft: (s) => set({ inputDraft: s }),

  send: async (text) => {
    if (get().sending) return;
    const parsed = parseCommand(text);
    if (parsed.kind === "noop") return;

    const taskStore = useTaskStore.getState();

    if (parsed.kind === "reset") {
      set({ inputDraft: "" });
      await get().reset();
      return;
    }
    if (parsed.kind === "help") {
      set((s) => ({
        inputDraft: "",
        blocks: [...s.blocks, {
          kind: "info", id: crypto.randomUUID(), taskId: "",
          content: HELP_TEXT,
        }],
      }));
      return;
    }
    if (parsed.kind === "status") {
      const tasks = taskStore.tasks;
      const completed = tasks.filter((t) => t.status === "completed").length;
      const running = tasks.filter((t) => t.status === "running" || !t.status).length;
      const cost = get().totalCost;
      const branch = useChangesStore.getState().status?.branch ?? "(none)";
      const status = [
        `session · ${taskStore.mode}`,
        `branch: ${branch}`,
        `tasks: ${tasks.length} (${completed} done, ${running} running)`,
        `cost so far: $${cost.toFixed(4)}`,
        `approve-all: ${get().approveAll ? "on" : "off"}`,
      ].join("\n");
      set((s) => ({
        inputDraft: "",
        blocks: [...s.blocks, {
          kind: "info", id: crypto.randomUUID(), taskId: "",
          content: status,
        }],
      }));
      return;
    }

    // parsed.kind === "send"
    if (parsed.mode) taskStore.setMode(parsed.mode);
    const body = parsed.body;
    if (!body) return;

    const task = taskStore.beginTask(body);
    set((s) => ({
      sending: true,
      inputDraft: "",
      hasMutatedThisTurn: false,
      blocks: [...s.blocks, { kind: "user", id: crypto.randomUUID(), taskId: task.id, content: body }],
    }));
    try {
      const snap = await ipc.chatSend(body, task.mode, task.constraints);
      if (snap) {
        taskStore.attachSnapshot(task.id, snap.commitSha);
      }
    } catch (err) {
      set({ sending: false });
      set((s) => ({
        blocks: [
          ...s.blocks,
          { kind: "text", id: crypto.randomUUID(), taskId: task.id, content: `error: ${err}` },
        ],
      }));
      useTaskStore.getState().completeTask(task.id, null, String(err));
    }
  },

  reset: async () => {
    try { await ipc.chatReset(); } catch { /* ignore */ }
    set({
      blocks: [], sending: false, totalCost: 0,
      pendingPermissions: [], hasMutatedThisTurn: false,
    });
    useTaskStore.getState().resetSession();
  },

  decidePermission: async (allow) => {
    const req = get().pendingPermissions[0];
    if (!req) return;
    set((s) => ({ pendingPermissions: s.pendingPermissions.slice(1) }));
    try {
      await ipc.permRespond(req.id, allow, allow ? null : "Denied by user");
    } catch { /* ignore */ }
  },

  setApproveAll: async (on) => {
    set({ approveAll: on });
    try { await ipc.permSetApproveAll(on); } catch { /* ignore */ }
    if (on) {
      while (get().pendingPermissions.length > 0) {
        await get().decidePermission(true);
      }
    }
  },

  restoreSnapshot: async (commitSha, userBlockId) => {
    try {
      await ipc.snapshotRestore(commitSha);
      set((s) => ({
        blocks: s.blocks.map((b) =>
          b.kind === "user" && b.id === userBlockId ? { ...b, restored: true } : b,
        ),
      }));
      await useChangesStore.getState().refresh();
    } catch (err) {
      set((s) => ({
        blocks: [
          ...s.blocks,
          { kind: "text", id: crypto.randomUUID(), taskId: "", content: `restore failed: ${err}` },
        ],
      }));
    }
  },

  _onText: (text) => {
    const taskId = useTaskStore.getState().currentTaskId ?? "";
    set((s) => ({
      blocks: [...s.blocks, { kind: "text", id: crypto.randomUUID(), taskId, content: text }],
    }));
  },

  _onToolUse: (e) => {
    const taskStore = useTaskStore.getState();
    const taskId = taskStore.currentTaskId ?? "";
    set((s) => ({
      blocks: [...s.blocks, {
        kind: "tool", id: e.id, taskId, name: e.name, input: e.input, status: "running",
      }],
    }));
    // Record into task aggregates immediately on tool_use; tool_result will confirm or mark error.
    const input = (e.input ?? {}) as Record<string, unknown>;
    const path = (typeof input.file_path === "string" ? input.file_path : null)
              ?? (typeof input.path === "string" ? input.path : null);
    if (e.name === "Bash" && typeof input.command === "string") {
      taskStore.recordCommand(input.command);
    } else if (MUTATING_TOOLS.has(e.name) && path) {
      taskStore.recordFileModified(path);
    } else if (READ_TOOLS.has(e.name) && path) {
      taskStore.recordFileRead(path);
    }
  },

  _onToolResult: (e) => {
    set((s) => ({
      blocks: s.blocks.map((b) =>
        b.kind === "tool" && b.id === e.toolUseId
          ? { ...b, status: e.ok ? "ok" : "error", summary: e.summary }
          : b,
      ),
    }));
    const block = get().blocks.find((b) => b.kind === "tool" && b.id === e.toolUseId);
    if (block && block.kind === "tool" && MUTATING_TOOLS.has(block.name)) {
      useChangesStore.getState().refresh().catch(() => { /* ignore */ });
      if (!get().hasMutatedThisTurn) {
        set((s) => ({ hasMutatedThisTurn: true, autoSwitchSignal: s.autoSwitchSignal + 1 }));
      }
    }
  },

  _onThinking: (e) => {
    const taskId = useTaskStore.getState().currentTaskId ?? "";
    set((s) => ({
      blocks: [...s.blocks, { kind: "thinking", id: crypto.randomUUID(), taskId, content: e.text }],
    }));
  },

  _onDone: (e) => {
    const taskStore = useTaskStore.getState();
    const taskId = taskStore.currentTaskId;
    set((s) => ({
      sending: false,
      totalCost: s.totalCost + (e.cost ?? 0),
    }));
    if (taskId) {
      taskStore.completeTask(taskId, e.cost, e.error);
    }
  },

  _onPerm: (r) => {
    if (get().approveAll) {
      ipc.permRespond(r.id, true).catch(() => {});
      return;
    }
    if (!DANGEROUS_TOOLS.has(r.toolName)) {
      ipc.permRespond(r.id, true).catch(() => {});
      return;
    }
    set((s) => ({ pendingPermissions: [...s.pendingPermissions, r] }));
  },
}));

export async function attachChatListeners(): Promise<UnlistenFn> {
  const s = useChatStore.getState();
  const offs: UnlistenFn[] = [];
  offs.push(await listen<ChatAssistantText>("chat://assistant-text", (e) => s._onText(e.payload.text)));
  offs.push(await listen<ChatToolUse>("chat://tool-use", (e) => s._onToolUse(e.payload)));
  offs.push(await listen<ChatToolResult>("chat://tool-result", (e) => s._onToolResult(e.payload)));
  offs.push(await listen<ChatThinking>("chat://thinking", (e) => s._onThinking(e.payload)));
  offs.push(await listen<ChatDone>("chat://done", (e) => s._onDone(e.payload)));
  offs.push(await listen<PermissionRequest>("perm://request", (e) => s._onPerm(e.payload)));
  offs.push(await listen("chat://session-ended", () => useChatStore.setState({ sending: false })));
  return () => { for (const off of offs) off(); };
}
