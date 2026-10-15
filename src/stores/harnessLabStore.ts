import { create } from "zustand";
import { ipc } from "../ipc/tauri";
import { useToastStore } from "./toastStore";
import type { HeldOutTask, LabConfig, LabSummary } from "../types/domain";

/// D12: the harness lab — what the console may change about its own
/// harness, measured by a metric the agent never computes, on tasks it
/// never sees. Proposes branches; a human merges.
interface HarnessLabState {
  project: string | null;
  summary: LabSummary | null;
  running: boolean;
  error: string | null;
  load: (projectRoot: string) => Promise<void>;
  saveConfig: (patch: Partial<LabConfig>) => Promise<void>;
  saveHeldout: (tasks: HeldOutTask[]) => Promise<void>;
  runNow: () => Promise<void>;
}

export const useHarnessLabStore = create<HarnessLabState>((set, get) => ({
  project: null,
  summary: null,
  running: false,
  error: null,
  load: async (projectRoot) => {
    try {
      set({ project: projectRoot, summary: await ipc.harnessLabGet(projectRoot), error: null });
    } catch (e) {
      set({ project: projectRoot, error: String(e) });
    }
  },
  saveConfig: async (patch) => {
    const { project, summary } = get();
    if (!project || !summary) return;
    try {
      const config = await ipc.harnessLabSetConfig(project, { ...summary.config, ...patch });
      set({ summary: { ...summary, config }, error: null });
    } catch (e) {
      set({ error: String(e) });
    }
  },
  saveHeldout: async (tasks) => {
    const { project, summary } = get();
    if (!project || !summary) return;
    try {
      const heldout = await ipc.harnessLabSetHeldout(project, tasks);
      set({ summary: { ...summary, heldout }, error: null });
    } catch (e) {
      set({ error: String(e) });
    }
  },
  runNow: async () => {
    const { project } = get();
    if (!project) return;
    set({ running: true, error: null });
    try {
      const exp = await ipc.harnessLabRun(project);
      useToastStore
        .getState()
        .show(
          `Experiment ${exp.id}: ${exp.status}${exp.stopReason ? ` — ${exp.stopReason}` : ""}${exp.prUrl ? ` — PR ready` : ""}`,
          exp.status === "kept" ? "success" : "info",
        );
      await get().load(project);
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set({ running: false });
    }
  },
}));
