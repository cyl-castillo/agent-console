import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { VaultEntryView } from "../types/domain";

interface VaultState {
  entries: VaultEntryView[];
  loading: boolean;
  error: string | null;

  refresh: () => Promise<void>;
  upsert: (params: {
    scope: "project" | "global";
    key: string;
    description: string;
    secret: boolean;
    value: string | null;
  }) => Promise<void>;
  remove: (scope: "project" | "global", key: string) => Promise<void>;
  reveal: (scope: "project" | "global", key: string) => Promise<string>;
}

export const useVaultStore = create<VaultState>((set, get) => ({
  entries: [],
  loading: false,
  error: null,

  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const entries = await ipc.vaultList();
      set({ entries, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  upsert: async (params) => {
    try {
      await ipc.vaultUpsert(params);
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },

  remove: async (scope, key) => {
    try {
      await ipc.vaultDelete(scope, key);
      await get().refresh();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  reveal: async (scope, key) => {
    return await ipc.vaultGetValue(scope, key);
  },
}));
