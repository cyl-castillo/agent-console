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

/// Tools whose completion can leave new diffs on disk → refresh Changes view.
const MUTATING_TOOLS = new Set([
  "Edit", "Write", "MultiEdit", "NotebookEdit", "Bash",
]);
/// Tools that should request the user's approval (the bundled hook will block until decided).
const DANGEROUS_TOOLS = new Set([
  "Edit", "Write", "MultiEdit", "NotebookEdit", "Bash",
]);

interface ChatState {
  blocks: ChatBlock[];
  sending: boolean;
  inputDraft: string;
  lastCost: number | null;
  totalCost: number;
  pendingPermissions: PermissionRequest[];
  approveAll: boolean;
  /// Bumped each time the agent first mutates files in the current turn —
  /// the UI uses this to auto-switch tabs once per turn.
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
  lastCost: null,
  totalCost: 0,
  pendingPermissions: [],
  approveAll: false,
  autoSwitchSignal: 0,
  hasMutatedThisTurn: false,

  setDraft: (s) => set({ inputDraft: s }),

  send: async (text) => {
    if (!text.trim() || get().sending) return;
    const userId = crypto.randomUUID();
    set((s) => ({
      sending: true,
      inputDraft: "",
      hasMutatedThisTurn: false,
      blocks: [...s.blocks, { kind: "user", id: userId, content: text }],
    }));
    try {
      const snap = await ipc.chatSend(text);
      if (snap) {
        set((s) => ({
          blocks: s.blocks.map((b) =>
            b.kind === "user" && b.id === userId ? { ...b, snapshot: snap } : b,
          ),
        }));
      }
    } catch (err) {
      set({ sending: false });
      set((s) => ({
        blocks: [
          ...s.blocks,
          { kind: "text", id: crypto.randomUUID(), content: `error: ${err}` },
        ],
      }));
    }
  },

  reset: async () => {
    try { await ipc.chatReset(); } catch { /* ignore */ }
    set({
      blocks: [], sending: false, lastCost: null, totalCost: 0,
      pendingPermissions: [], hasMutatedThisTurn: false,
    });
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
      // Drain any queued requests as approved.
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
          { kind: "text", id: crypto.randomUUID(), content: `restore failed: ${err}` },
        ],
      }));
    }
  },

  _onText: (text) => set((s) => ({
    blocks: [...s.blocks, { kind: "text", id: crypto.randomUUID(), content: text }],
  })),

  _onToolUse: (e) => {
    set((s) => ({
      blocks: [...s.blocks, {
        kind: "tool", id: e.id, name: e.name, input: e.input, status: "running",
      }],
    }));
    // Dangerous tools will be paused by the hook; for safe ones the request never fires.
    // We don't need to act here — the perm:// event drives the modal.
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

  _onThinking: (e) => set((s) => ({
    blocks: [...s.blocks, { kind: "thinking", id: crypto.randomUUID(), content: e.text }],
  })),

  _onDone: (e) => set((s) => ({
    sending: false,
    lastCost: e.cost,
    totalCost: s.totalCost + (e.cost ?? 0),
  })),

  _onPerm: (r) => {
    // If approve-all is on, auto-accept immediately.
    if (get().approveAll) {
      ipc.permRespond(r.id, true).catch(() => {});
      return;
    }
    // Some hooks may fire for safe tools if matchers loosen; double-check.
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
