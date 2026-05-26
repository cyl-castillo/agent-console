import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { PermissionsSnapshot, StoredRule } from "../types/domain";

type Scope = "project" | "global";
type Effect = "allow" | "deny" | "ask";

interface LastOp {
  kind: "add" | "remove";
  scope: Scope;
  effect: Effect;
  raw: string;
}

interface PermissionsState {
  snapshot: PermissionsSnapshot | null;
  loading: boolean;
  lastOp: LastOp | null;
  error: string | null;

  refresh: () => Promise<void>;
  add: (scope: Scope, effect: Effect, raw: string, track?: boolean) => Promise<StoredRule | null>;
  remove: (scope: Scope, effect: Effect, raw: string, track?: boolean) => Promise<void>;
  move: (from: StoredRule, toScope: Scope) => Promise<void>;
  undo: () => Promise<void>;
  clearError: () => void;
}

export const usePermissionsStore = create<PermissionsState>((set, get) => ({
  snapshot: null,
  loading: false,
  lastOp: null,
  error: null,

  refresh: async () => {
    set({ loading: true });
    try {
      const snap = await ipc.permissionsSnapshot();
      set({ snapshot: snap });
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set({ loading: false });
    }
  },

  add: async (scope, effect, raw, track = true) => {
    try {
      const r = await ipc.permissionsAdd(scope, effect, raw);
      if (track) set({ lastOp: { kind: "add", scope, effect, raw } });
      await get().refresh();
      return r;
    } catch (e) {
      set({ error: String(e) });
      return null;
    }
  },

  remove: async (scope, effect, raw, track = true) => {
    try {
      await ipc.permissionsRemove(scope, effect, raw);
      if (track) set({ lastOp: { kind: "remove", scope, effect, raw } });
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  // Move = remove from old scope + add to new scope. Tracks as a single
  // 'add' op (the more interesting half) so undo removes the new placement.
  move: async (from, toScope) => {
    if (from.scope === toScope) return;
    try {
      await ipc.permissionsAdd(toScope, from.effect, from.raw);
      await ipc.permissionsRemove(from.scope, from.effect, from.raw);
      set({ lastOp: { kind: "add", scope: toScope, effect: from.effect, raw: from.raw } });
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  undo: async () => {
    const op = get().lastOp;
    if (!op) return;
    set({ lastOp: null });
    if (op.kind === "add") {
      await get().remove(op.scope, op.effect, op.raw, false);
    } else {
      await get().add(op.scope, op.effect, op.raw, false);
    }
  },

  clearError: () => set({ error: null }),
}));
