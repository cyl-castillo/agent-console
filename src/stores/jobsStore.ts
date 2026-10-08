import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import { useRoundtableStore } from "./roundtableStore";
import type { JobsBoard, JobsChanged } from "../types/domain";

/// The project's jobs board (port of ai-connector's job manager). The backend
/// owns every transition; this store only fetches the board and forwards the
/// human's actions, refetching on `roundtable://jobs`.
interface JobsState {
  board: JobsBoard | null;
  loading: boolean;
  error: string | null;
  /// Card id with an action in flight (buttons disable).
  busyId: string | null;

  initListeners: () => Promise<void>;
  load: () => Promise<void>;
  startNow: (id: string) => Promise<void>;
  continueJob: (id: string) => Promise<void>;
  closeJob: (id: string) => Promise<void>;
  move: (id: string, up: boolean) => Promise<void>;
  confirmLanding: (id: string) => Promise<void>;
  setParallel: (n: number) => Promise<void>;
  /// Approve (start as a job room) or discard a `create_task` proposal.
  resolvePending: (sourceId: string, pendingId: string, approve: boolean) => Promise<void>;
}

let bindPromise: Promise<void> | null = null;
let unlistenJobs: UnlistenFn | null = null;

export const useJobsStore = create<JobsState>((set, get) => {
  const run = async (id: string | null, f: () => Promise<void>) => {
    set({ busyId: id, error: null });
    try {
      await f();
    } catch (err) {
      set({ error: err instanceof Error ? err.message : String(err) });
    } finally {
      set({ busyId: null });
    }
    await get().load();
  };

  return {
    board: null,
    loading: false,
    error: null,
    busyId: null,

    initListeners: () => {
      bindPromise ??= (async () => {
        unlistenJobs = await listen<JobsChanged>("roundtable://jobs", (e) => {
          const project = get().board?.project;
          if (project && e.payload.project !== project) return;
          void get().load();
        });
      })();
      return bindPromise;
    },

    load: async () => {
      set({ loading: true });
      try {
        set({ board: await ipc.jobsBoard(), error: null });
      } catch (err) {
        set({ error: err instanceof Error ? err.message : String(err) });
      } finally {
        set({ loading: false });
      }
    },

    startNow: (id) => run(id, () => ipc.jobStartNow(id)),
    continueJob: (id) => run(id, () => ipc.jobContinue(id)),
    closeJob: (id) => run(id, () => ipc.jobClose(id)),
    move: (id, up) => run(id, () => ipc.jobMove(id, up)),
    confirmLanding: (id) => run(id, () => ipc.jobConfirmLanding(id)),
    setParallel: (n) => run(null, () => ipc.jobsSetParallel(n)),

    resolvePending: (sourceId, pendingId, approve) =>
      run(pendingId, async () => {
        await ipc.roundtableResolvePending(sourceId, pendingId, approve);
        // The saved-room list grows with the follow-up room.
        void useRoundtableStore.getState().loadRooms();
      }),
  };
});

export async function teardownJobsListeners() {
  const inFlight = bindPromise;
  bindPromise = null;
  if (inFlight) await inFlight.catch(() => {});
  unlistenJobs?.();
  unlistenJobs = null;
}
