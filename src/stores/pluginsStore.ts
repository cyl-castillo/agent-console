import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import { useTerminalsStore } from "./terminalsStore";
import type { InstalledPlugin, MarketplacePlugin, MarketplaceSnapshot } from "../types/domain";

interface PluginsState {
  installed: InstalledPlugin[];
  marketplace: MarketplacePlugin[];
  marketplaceSource: string | null;
  marketplaceFetchedAtMs: number | null;
  marketplaceIsFallback: boolean;
  query: string;
  installedLoading: boolean;
  marketplaceLoading: boolean;
  error: string | null;

  setQuery: (q: string) => void;
  refreshInstalled: () => Promise<void>;
  refreshMarketplace: (force?: boolean) => Promise<void>;
  installViaTerminal: (slug: string) => string | null;
}

export const usePluginsStore = create<PluginsState>((set) => ({
  installed: [],
  marketplace: [],
  marketplaceSource: null,
  marketplaceFetchedAtMs: null,
  marketplaceIsFallback: false,
  query: "",
  installedLoading: false,
  marketplaceLoading: false,
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

  refreshMarketplace: async (force) => {
    set({ marketplaceLoading: true, error: null });
    try {
      const snap: MarketplaceSnapshot = await ipc.pluginsMarketplace(!!force);
      set({
        marketplace: snap.plugins,
        marketplaceSource: snap.source,
        marketplaceFetchedAtMs: snap.fetchedAtMs,
        marketplaceIsFallback: snap.isFallback,
        marketplaceLoading: false,
      });
    } catch (e) {
      set({ marketplaceLoading: false, error: String(e) });
    }
  },

  installViaTerminal: (slug) => {
    const term = useTerminalsStore.getState();
    const activeId = term.activeId;
    const session = term.sessions.find((s) => s.id === activeId);
    if (!session) return "no-active-session";
    // Switch to terminal tab so the user sees the command land.
    window.dispatchEvent(new CustomEvent("ac:open-tab", { detail: "terminal" }));
    // Write the slash command + Enter.
    const cmd = `/plugin install ${slug}\r`;
    void import("../ipc/tauri").then(({ ipc }) => ipc.termWrite(session.id, cmd).catch(() => {}));
    return null;
  },
}));
