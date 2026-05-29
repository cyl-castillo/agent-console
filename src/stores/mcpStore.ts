import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { McpServer, McpAddInput } from "../types/domain";

interface McpState {
  servers: McpServer[];
  loading: boolean;
  error: string | null;
  /// names currently being removed (drives per-row spinner).
  removing: Record<string, true>;
  adding: boolean;
  addError: string | null;

  refresh: () => Promise<void>;
  add: (input: McpAddInput) => Promise<boolean>;
  remove: (name: string, scope: string) => Promise<void>;
  clearAddError: () => void;
}

export const useMcpStore = create<McpState>((set, get) => ({
  servers: [],
  loading: false,
  error: null,
  removing: {},
  adding: false,
  addError: null,

  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const servers = await ipc.mcpList();
      set({ servers, loading: false });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  add: async (input) => {
    set({ adding: true, addError: null });
    try {
      await ipc.mcpAdd(input);
      set({ adding: false });
      await get().refresh();
      return true;
    } catch (e) {
      set({ adding: false, addError: String(e) });
      return false;
    }
  },

  remove: async (name, scope) => {
    if (get().removing[name]) return;
    set((s) => ({ removing: { ...s.removing, [name]: true } }));
    try {
      await ipc.mcpRemove(name, scope);
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set((s) => {
        const removing = { ...s.removing };
        delete removing[name];
        return { removing };
      });
    }
  },

  clearAddError: () => set({ addError: null }),
}));
