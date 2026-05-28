import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type { InstalledPlugin, MarketplacePlugin } from "../types/domain";

export type PluginScope = "user" | "project" | "local";

interface PluginsState {
  installed: InstalledPlugin[];
  available: MarketplacePlugin[];
  marketplaces: string[];
  query: string;
  installedLoading: boolean;
  availableLoading: boolean;
  /// install ids currently being installed (drives per-row spinners).
  installing: Record<string, true>;
  /// last install error keyed by install id.
  installErrors: Record<string, string>;
  error: string | null;

  setQuery: (q: string) => void;
  refreshInstalled: () => Promise<void>;
  refreshAvailable: () => Promise<void>;
  install: (installId: string, scope?: PluginScope) => Promise<void>;
}

export const usePluginsStore = create<PluginsState>((set, get) => ({
  installed: [],
  available: [],
  marketplaces: [],
  query: "",
  installedLoading: false,
  availableLoading: false,
  installing: {},
  installErrors: {},
  error: null,

  setQuery: (q) => set({ query: q }),

  refreshInstalled: async () => {
    set({ installedLoading: true, error: null });
    try {
      const installed = await ipc.pluginsListInstalled();
      set({ installed, installedLoading: false });
    } catch (e) {
      set({ installedLoading: false, error: String(e) });
    }
  },

  refreshAvailable: async () => {
    set({ availableLoading: true, error: null });
    try {
      const snap = await ipc.pluginsListAvailable();
      set({
        available: snap.plugins,
        marketplaces: snap.marketplaces,
        availableLoading: false,
      });
    } catch (e) {
      set({ availableLoading: false, error: String(e) });
    }
  },

  install: async (installId, scope = "user") => {
    if (get().installing[installId]) return;
    set((s) => ({
      installing: { ...s.installing, [installId]: true },
      installErrors: { ...s.installErrors, [installId]: "" },
    }));
    try {
      await ipc.pluginsInstall(installId, scope);
      // Refresh the installed list; the row moves out of "available" since the
      // panel filters out already-installed slugs.
      await get().refreshInstalled();
      set((s) => {
        const installing = { ...s.installing };
        delete installing[installId];
        const installErrors = { ...s.installErrors };
        delete installErrors[installId];
        return { installing, installErrors };
      });
    } catch (e) {
      set((s) => {
        const installing = { ...s.installing };
        delete installing[installId];
        return {
          installing,
          installErrors: { ...s.installErrors, [installId]: String(e) },
        };
      });
    }
  },
}));
