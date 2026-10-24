import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ipc } from "../ipc/tauri";
import type { TrustStatus } from "../types/domain";
import { useSessionStore } from "./sessionStore";

/// F4: the open folder's agent surface and the human's word on it. Loaded
/// when a project opens, re-read when the Rust side announces a decision
/// (`console://trust`) so the Inbox card and the waiting terminals move.
interface TrustState {
  projectRoot: string | null;
  status: TrustStatus | null;
  loading: boolean;
  error: string | null;
  load: (projectRoot: string) => Promise<void>;
  decide: (decision: "trusted" | "declined") => Promise<void>;
}

export const useTrustStore = create<TrustState>((set, get) => ({
  projectRoot: null,
  status: null,
  loading: false,
  error: null,
  load: async (projectRoot) => {
    set({ projectRoot, loading: true, error: null });
    try {
      const status = await ipc.trustStatus(projectRoot);
      if (get().projectRoot !== projectRoot) return;
      set({ status, loading: false });
    } catch (e) {
      if (get().projectRoot !== projectRoot) return;
      set({ loading: false, error: String(e) });
    }
  },
  decide: async (decision) => {
    const root = get().projectRoot;
    if (!root) return;
    const status = await ipc.trustDecide(root, decision);
    if (get().projectRoot === root) set({ status });
  },
}));

/// Follow the open project and the Rust side's announcements.
export async function attachTrustListeners(): Promise<UnlistenFn> {
  const unsubProject = useSessionStore.subscribe((s, prev) => {
    const root = s.project?.root ?? null;
    if (root && root !== (prev.project?.root ?? null)) void useTrustStore.getState().load(root);
    if (!root) useTrustStore.setState({ projectRoot: null, status: null });
  });
  const root = useSessionStore.getState().project?.root;
  if (root) void useTrustStore.getState().load(root);
  const unlisten = await listen<{ project: string; state: string }>("console://trust", (e) => {
    const cur = useTrustStore.getState().projectRoot;
    if (cur && e.payload?.project === cur) void useTrustStore.getState().load(cur);
  });
  return () => {
    unsubProject();
    unlisten();
  };
}
