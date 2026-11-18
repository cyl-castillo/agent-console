import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import { isTabEnabled } from "./modulesStore";
import { useToastStore } from "./toastStore";
import type { ProofEvent, WatchExchange, WatchGoal, WatchSource } from "../types/domain";

export type WatchStatus = "idle" | "loading" | "ready" | "error";

interface WatchState {
  goals: WatchGoal[];
  source: WatchSource | null;
  status: WatchStatus;
  errorMessage: string | null;
  /// The alert the human has open: what "that run" refers to in a question.
  focus: { goalId: string; alertId: string } | null;
  /// Ledger lines behind the focused alert.
  evidence: ProofEvent[];
  evidenceError: string | null;
  asking: boolean;

  refresh: () => Promise<void>;
  start: (title: string, demo: boolean) => Promise<void>;
  setStatus: (id: string, status: "active" | "paused" | "stopped") => Promise<void>;
  remove: (id: string) => Promise<void>;
  openAlert: (goalId: string, alertId: string) => Promise<void>;
  closeAlert: () => void;
  dismissAlert: (goalId: string, alertId: string) => Promise<void>;
  ask: (goalId: string, question: string) => Promise<WatchExchange | null>;
}

const message = (err: unknown) => (err instanceof Error ? err.message : String(err));

function replaceGoal(goals: WatchGoal[], goal: WatchGoal): WatchGoal[] {
  return goals.map((g) => (g.id === goal.id ? goal : g));
}

export const useWatchStore = create<WatchState>((set, get) => ({
  goals: [],
  source: null,
  status: "idle",
  errorMessage: null,
  focus: null,
  evidence: [],
  evidenceError: null,
  asking: false,

  refresh: async () => {
    if (get().status === "idle") set({ status: "loading" });
    try {
      const [goals, source] = await Promise.all([ipc.watchList(), ipc.watchSource()]);
      // A focused alert that no longer exists (goal deleted) closes itself.
      const focus = get().focus;
      const alive =
        focus &&
        goals.some((g) => g.id === focus.goalId && g.alerts.some((a) => a.id === focus.alertId));
      set({
        goals,
        source,
        status: "ready",
        errorMessage: null,
        ...(alive ? {} : { focus: null, evidence: [], evidenceError: null }),
      });
    } catch (err) {
      set({ status: "error", errorMessage: message(err) });
    }
  },

  start: async (title, demo) => {
    await ipc.watchStart(title, demo);
    await get().refresh();
  },

  setStatus: async (id, status) => {
    const goal = await ipc.watchSetStatus(id, status);
    set((s) => ({ goals: replaceGoal(s.goals, goal) }));
  },

  remove: async (id) => {
    await ipc.watchDelete(id);
    await get().refresh();
  },

  openAlert: async (goalId, alertId) => {
    set({ focus: { goalId, alertId }, evidence: [], evidenceError: null });
    try {
      const alert = get()
        .goals.find((g) => g.id === goalId)
        ?.alerts.find((a) => a.id === alertId);
      if (alert?.state === "new") {
        const goal = await ipc.watchSetAlertState(goalId, alertId, "seen");
        set((s) => ({ goals: replaceGoal(s.goals, goal) }));
      }
      const evidence = await ipc.watchEvidence(goalId, alertId);
      // Ignore a late answer for an alert that is no longer the open one.
      if (get().focus?.alertId === alertId) set({ evidence });
    } catch (err) {
      if (get().focus?.alertId === alertId) set({ evidenceError: message(err) });
    }
  },

  closeAlert: () => set({ focus: null, evidence: [], evidenceError: null }),

  dismissAlert: async (goalId, alertId) => {
    const goal = await ipc.watchSetAlertState(goalId, alertId, "dismissed");
    set((s) => ({
      goals: replaceGoal(s.goals, goal),
      ...(s.focus?.alertId === alertId ? { focus: null, evidence: [], evidenceError: null } : {}),
    }));
  },

  ask: async (goalId, question) => {
    set({ asking: true });
    try {
      const focus = get().focus;
      const ex = await ipc.watchAsk(
        goalId,
        question,
        focus?.goalId === goalId ? focus.alertId : null,
      );
      set((s) => ({
        goals: s.goals.map((g) => (g.id === goalId ? { ...g, thread: [...g.thread, ex] } : g)),
      }));
      return ex;
    } catch (err) {
      useToastStore.getState().show(`Watch: ${message(err)}`, "error");
      return null;
    } finally {
      set({ asking: false });
    }
  },
}));

/// Alerts nobody has opened yet, across the project's goals.
export function openAlertCount(s: Pick<WatchState, "goals">): number {
  return s.goals.reduce((n, g) => n + g.alerts.filter((a) => a.state === "new").length, 0);
}

function openWatch(goalId: string, alertId: string) {
  window.dispatchEvent(new CustomEvent("ac:open-workbench-tab", { detail: "watch" }));
  void useWatchStore.getState().openAlert(goalId, alertId);
}

interface AlertEvent {
  projectRoot: string;
  goalId: string;
  alertId: string;
  command: string;
  recovered: boolean;
}

/// Subscribe to the backend watch events. The OS notification is the Rust
/// side's job (it knows whether the window has focus and sends at most one
/// per alert); here the panel refreshes and, with the module on, a toast
/// leads to the evidence.
export async function attachWatchListeners(): Promise<UnlistenFn> {
  const offs: UnlistenFn[] = [];
  offs.push(await listen("watch://changed", () => void useWatchStore.getState().refresh()));
  offs.push(
    await listen<AlertEvent>("watch://alert", (e) => {
      if (!isTabEnabled("watch")) return;
      const { goalId, alertId, command, recovered } = e.payload;
      useToastStore
        .getState()
        .show(
          recovered ? `Failed while the app was closed: ${command}` : `A check failed: ${command}`,
          "info",
          {
            action: { label: "Evidence", onClick: () => openWatch(goalId, alertId) },
            timeoutMs: 12_000,
          },
        );
    }),
  );
  return () => {
    for (const off of offs) off();
  };
}
