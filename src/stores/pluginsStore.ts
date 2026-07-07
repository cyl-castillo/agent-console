import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import { useToastStore } from "./toastStore";
import type { InstalledPlugin, MarketplacePlugin } from "../types/domain";

export type PluginScope = "user" | "project" | "local";

/// The scope the CLI reported for an installed plugin, normalized to what
/// `plugin update --scope` accepts (anything else falls back to user).
function scopeOf(p: InstalledPlugin): PluginScope {
  return p.scope === "project" || p.scope === "local" ? p.scope : "user";
}

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
  /// plugin ids currently being updated (drives per-row spinners).
  updating: Record<string, true>;
  /// last update error keyed by plugin id.
  updateErrors: Record<string, string>;
  /// true while "Update all" runs (refresh marketplaces + every plugin).
  updatingAll: boolean;
  error: string | null;

  setQuery: (q: string) => void;
  refreshInstalled: () => Promise<void>;
  refreshAvailable: () => Promise<void>;
  install: (installId: string, scope?: PluginScope) => Promise<void>;
  update: (id: string) => Promise<boolean>;
  updateAll: () => Promise<void>;
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
  updating: {},
  updateErrors: {},
  updatingAll: false,
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

  update: async (id) => {
    if (get().updating[id]) return false;
    const plugin = get().installed.find((p) => p.id === id);
    set((s) => ({
      updating: { ...s.updating, [id]: true },
      updateErrors: { ...s.updateErrors, [id]: "" },
    }));
    try {
      await ipc.pluginsUpdate(id, plugin ? scopeOf(plugin) : "user");
      await get().refreshInstalled();
      set((s) => {
        const updating = { ...s.updating };
        delete updating[id];
        const updateErrors = { ...s.updateErrors };
        delete updateErrors[id];
        return { updating, updateErrors };
      });
      return true;
    } catch (e) {
      set((s) => {
        const updating = { ...s.updating };
        delete updating[id];
        return {
          updating,
          updateErrors: { ...s.updateErrors, [id]: String(e) },
        };
      });
      return false;
    }
  },

  updateAll: async () => {
    if (get().updatingAll) return;
    set({ updatingAll: true });
    try {
      // Refresh the marketplaces first so updates resolve against the latest
      // catalogue; a failure here isn't fatal (update still works per-plugin).
      await ipc.pluginsUpdateMarketplaces().catch(() => {});
      const targets = get().installed;
      let ok = 0;
      let failed = 0;
      for (const p of targets) {
        // Sequential on purpose: the CLI mutates shared config; racing N
        // installs of it invites corruption. Keep going past failures.
        if (await get().update(p.id)) ok++;
        else failed++;
      }
      const toast = useToastStore.getState();
      if (failed === 0) {
        toast.show(
          ok === 0
            ? "No plugins installed"
            : `${ok} plugin${ok === 1 ? "" : "s"} updated — restart agent sessions to apply`,
          "success",
        );
      } else {
        toast.show(`${ok} updated, ${failed} failed — see the rows below`, "error");
      }
    } finally {
      set({ updatingAll: false });
    }
  },
}));
