import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ipc } from "../ipc/tauri";
import type { FanOut, FanOutConfig, MergeOutcome } from "../types/domain";

/// F9: fan-outs of the open project — N agents on the same prompt in twin
/// worktrees, compared, one applied by the human. Re-read on
/// `fanout://changed` (a candidate finished, a pick landed).
interface FanOutState {
  items: FanOut[];
  loading: boolean;
  error: string | null;
  load: () => Promise<void>;
  start: (config: FanOutConfig) => Promise<string>;
  apply: (id: string, participantId: string, reason?: string) => Promise<MergeOutcome>;
  discard: (id: string) => Promise<void>;
}

export const useFanOutStore = create<FanOutState>((set, get) => ({
  items: [],
  loading: false,
  error: null,
  load: async () => {
    set({ loading: true, error: null });
    try {
      const items = await ipc.fanoutList();
      set({ items: items ?? [], loading: false });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },
  start: async (config) => {
    const id = await ipc.fanoutStart(config);
    await get().load();
    return id;
  },
  apply: async (id, participantId, reason) => {
    const out = await ipc.fanoutApply(id, participantId, reason);
    await get().load();
    return out;
  },
  discard: async (id) => {
    await ipc.fanoutDiscard(id);
    await get().load();
  },
}));

export async function attachFanOutListener(): Promise<UnlistenFn> {
  return listen("fanout://changed", () => {
    void useFanOutStore.getState().load();
  });
}
