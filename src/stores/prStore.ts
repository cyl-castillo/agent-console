import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ipc } from "../ipc/tauri";
import type { PrOverview } from "../types/domain";
import { useSessionStore } from "./sessionStore";

/// F7: the checked-out branch's pull request as the ledger sees it — CI,
/// comments and the evidence behind every commit. Loaded when a project
/// opens, re-read when the Rust side announces a merge (`console://pr`),
/// when the ledger gains a commit, and on a slow clock while CI runs.
interface PrState {
  projectRoot: string | null;
  overview: PrOverview | null;
  loading: boolean;
  error: string | null;
  loadedAt: number | null;
  /// `${number}:${headSha}` the human set aside ("Later") this session.
  dismissed: string[];
  load: (projectRoot: string) => Promise<void>;
  refresh: () => Promise<void>;
  dismiss: (key: string) => void;
}

/// CI still running: ask again soon. Otherwise a slow heartbeat so a review
/// that lands while you work shows up without a click.
export const PR_POLL_PENDING_MS = 60_000;
export const PR_POLL_IDLE_MS = 5 * 60_000;

export const usePrStore = create<PrState>((set, get) => ({
  projectRoot: null,
  overview: null,
  loading: false,
  error: null,
  loadedAt: null,
  dismissed: [],
  load: async (projectRoot) => {
    if (get().loading) return;
    set({ projectRoot, loading: true, error: null });
    try {
      const overview = await ipc.prStatus();
      if (get().projectRoot !== projectRoot) return;
      set({ overview, loading: false, loadedAt: Date.now() });
    } catch (e) {
      if (get().projectRoot !== projectRoot) return;
      set({ loading: false, error: String(e), loadedAt: Date.now() });
    }
  },
  refresh: async () => {
    const root = get().projectRoot;
    if (root) await get().load(root);
  },
  dismiss: (key) => set((s) => ({ dismissed: [...s.dismissed, key] })),
}));

export function prKey(ov: PrOverview | null): string | null {
  return ov?.pr ? `${ov.pr.number}:${ov.pr.headSha}` : null;
}

/// Follow the open project, the Rust side's announcements and the clock.
export async function attachPrListeners(): Promise<UnlistenFn> {
  const unsubProject = useSessionStore.subscribe((s, prev) => {
    const root = s.project?.root ?? null;
    if (root && root !== (prev.project?.root ?? null)) void usePrStore.getState().load(root);
    if (!root) usePrStore.setState({ projectRoot: null, overview: null });
  });
  const root = useSessionStore.getState().project?.root;
  if (root) void usePrStore.getState().load(root);
  const onPr = await listen<{ project: string }>("console://pr", () => {
    void usePrStore.getState().refresh();
  });
  // A commit (GUI or agent) changes the evidence; a push changes the PR.
  const onLedger = await listen<{ kind?: string }>("ledger://appended", (e) => {
    if (e.payload?.kind === "commit") void usePrStore.getState().refresh();
  });
  const tick = window.setInterval(() => {
    const { overview, loadedAt, projectRoot } = usePrStore.getState();
    if (!projectRoot || loadedAt === null) return;
    const period = overview?.pr?.ci === "pending" ? PR_POLL_PENDING_MS : PR_POLL_IDLE_MS;
    if (Date.now() - loadedAt >= period) void usePrStore.getState().refresh();
  }, 15_000);
  return () => {
    unsubProject();
    onPr();
    onLedger();
    window.clearInterval(tick);
  };
}
