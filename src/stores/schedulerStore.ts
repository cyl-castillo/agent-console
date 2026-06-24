import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import type { Job, RunRecord } from "../types/domain";

export type SchedulerStatus = "idle" | "loading" | "ready" | "error";

/// The curated set of app events that event-triggered jobs can listen for. The
/// app emits these from the matching places (a commit landing, the corpus
/// growing, a prompt being sent); the editor offers exactly this list.
export const SCHEDULER_EVENTS: ReadonlyArray<{ name: string; label: string }> = [
  { name: "commit", label: "a commit lands" },
  { name: "corpus_grew", label: "a skill or memory is added" },
  { name: "prompt", label: "you send a prompt" },
];

/// Fire any event-triggered jobs for `name`. Best-effort: with no project open
/// (or on a transient failure) this is a no-op, never a throw into a hot path
/// like the prompt or commit flow. The backend returns immediately and offloads
/// the actual runs, and is cheap when no job listens for the event.
export async function fireSchedulerEvent(name: string): Promise<void> {
  try {
    await ipc.schedulerFireEvent(name);
  } catch {
    /* ignore — no project, or no matching job */
  }
}

interface SchedulerState {
  jobs: Job[];
  history: RunRecord[];
  /// Ids of jobs currently executing (driven by scheduler://run_* events, so
  /// both manual run-now and the background tick loop light up the same way).
  runningJobIds: string[];
  status: SchedulerStatus;
  errorMessage: string | null;

  refresh: () => Promise<void>;
  refreshHistory: () => Promise<void>;
  createJob: (job: Job) => Promise<Job>;
  updateJob: (job: Job) => Promise<Job>;
  deleteJob: (id: string) => Promise<void>;
  setEnabled: (id: string, enabled: boolean) => Promise<void>;
  runNow: (id: string) => Promise<void>;

  // event handlers (wired by attachSchedulerListeners)
  _onRunStarted: (jobId: string) => void;
  _onRunFinished: (rec: RunRecord) => void;
  _onJobsChanged: () => void;
}

export const useSchedulerStore = create<SchedulerState>((set, get) => ({
  jobs: [],
  history: [],
  runningJobIds: [],
  status: "idle",
  errorMessage: null,

  refresh: async () => {
    set({ status: "loading", errorMessage: null });
    try {
      const [jobs, history] = await Promise.all([
        ipc.schedulerList(),
        ipc.schedulerHistory(),
      ]);
      set({ jobs, history, status: "ready" });
    } catch (err) {
      set({
        status: "error",
        errorMessage: err instanceof Error ? err.message : String(err),
      });
    }
  },

  refreshHistory: async () => {
    try {
      const history = await ipc.schedulerHistory();
      set({ history });
    } catch {
      /* a transient read failure shouldn't blank the panel */
    }
  },

  createJob: async (job) => {
    const created = await ipc.schedulerCreate(job);
    await get().refresh();
    return created;
  },

  updateJob: async (job) => {
    const updated = await ipc.schedulerUpdate(job);
    await get().refresh();
    return updated;
  },

  deleteJob: async (id) => {
    await ipc.schedulerDelete(id);
    await get().refresh();
  },

  setEnabled: async (id, enabled) => {
    // Optimistic flip so the toggle feels instant; reconcile from the result.
    set((s) => ({
      jobs: s.jobs.map((j) => (j.id === id ? { ...j, enabled } : j)),
    }));
    try {
      const updated = await ipc.schedulerSetEnabled(id, enabled);
      set((s) => ({ jobs: s.jobs.map((j) => (j.id === id ? updated : j)) }));
    } catch (err) {
      await get().refresh(); // revert to server truth
      throw err;
    }
  },

  runNow: async (id) => {
    // The run_started/run_finished events handle the spinner + history; here we
    // just kick it off and let the result land through the event stream.
    await ipc.schedulerRunNow(id);
    await get().refresh();
  },

  _onRunStarted: (jobId) => {
    set((s) =>
      s.runningJobIds.includes(jobId)
        ? s
        : { runningJobIds: [...s.runningJobIds, jobId] },
    );
  },

  _onRunFinished: (rec) => {
    set((s) => ({
      runningJobIds: s.runningJobIds.filter((id) => id !== rec.jobId),
      history: [rec, ...s.history].slice(0, 200),
    }));
    // next_due/last_run changed on the backend — pull fresh job rows.
    void get().refresh();
  },

  _onJobsChanged: () => {
    void get().refresh();
  },
}));

/// Subscribe to the backend scheduler event stream. Mirrors
/// attachSkillsListeners: returns one unlisten that detaches everything.
export async function attachSchedulerListeners(): Promise<UnlistenFn> {
  const s = useSchedulerStore.getState();
  const offs: UnlistenFn[] = [];
  offs.push(
    await listen<{ jobId: string }>("scheduler://run_started", (e) =>
      s._onRunStarted(e.payload.jobId),
    ),
  );
  offs.push(
    await listen<RunRecord>("scheduler://run_finished", (e) =>
      s._onRunFinished(e.payload),
    ),
  );
  offs.push(
    await listen("scheduler://jobs_changed", () => s._onJobsChanged()),
  );
  return () => {
    for (const off of offs) off();
  };
}
