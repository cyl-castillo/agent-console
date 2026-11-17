import { beforeEach, describe, expect, it, vi } from "vitest";

import type { WatchAlert, WatchGoal } from "../types/domain";

const ipc = vi.hoisted(() => ({
  watchList: vi.fn(),
  watchSource: vi.fn(),
  watchSetAlertState: vi.fn(),
  watchEvidence: vi.fn(),
  watchAsk: vi.fn(),
}));
vi.mock("../ipc/tauri", () => ({ ipc }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("./toastStore", () => ({ useToastStore: { getState: () => ({ show: vi.fn() }) } }));

import { openAlertCount, useWatchStore } from "./watchStore";

const alert = (id: string, state: string): WatchAlert => ({
  id,
  condition: "check_failed",
  key: id,
  ts: 1,
  lastTs: 1,
  caseId: "term:t1",
  command: "npm test",
  checkKind: "test",
  evidenceSeqs: [3],
  state,
  notified: true,
  recovered: false,
});

const goal = (alerts: WatchAlert[]): WatchGoal => ({
  id: "g1",
  title: "Close the PR today",
  source: "ledger",
  status: "active",
  conditions: ["check_failed"],
  createdMs: 0,
  cursorSeq: 3,
  alerts,
  thread: [],
});

beforeEach(() => {
  vi.clearAllMocks();
  useWatchStore.setState({
    goals: [goal([alert("a1", "new"), alert("a2", "seen"), alert("a3", "dismissed")])],
    focus: null,
    evidence: [],
    evidenceError: null,
    status: "ready",
  });
});

describe("watch store", () => {
  it("counts only the alerts nobody opened", () => {
    expect(openAlertCount(useWatchStore.getState())).toBe(1);
  });

  it("opening a new alert marks it seen, focuses it and loads its ledger lines", async () => {
    ipc.watchSetAlertState.mockResolvedValue(goal([alert("a1", "seen")]));
    ipc.watchEvidence.mockResolvedValue([{ seq: 3, kind: "check_run" }]);
    await useWatchStore.getState().openAlert("g1", "a1");
    expect(ipc.watchSetAlertState).toHaveBeenCalledWith("g1", "a1", "seen");
    const s = useWatchStore.getState();
    expect(s.focus).toEqual({ goalId: "g1", alertId: "a1" });
    expect(s.evidence).toHaveLength(1);
    expect(openAlertCount(s)).toBe(0);
  });

  it("an unreadable ledger is reported on the alert, not shown as empty evidence", async () => {
    ipc.watchEvidence.mockRejectedValue(new Error("io: gone"));
    await useWatchStore.getState().openAlert("g1", "a2");
    expect(ipc.watchSetAlertState).not.toHaveBeenCalled();
    expect(useWatchStore.getState().evidenceError).toBe("io: gone");
  });

  it("a question carries the open alert as what 'that run' means", async () => {
    ipc.watchAsk.mockResolvedValue({
      question: "q",
      answer: "a",
      evidenceSeqs: [],
      candidates: [],
    });
    await useWatchStore.getState().ask("g1", "what happened in that run?");
    expect(ipc.watchAsk).toHaveBeenLastCalledWith("g1", "what happened in that run?", null);
    useWatchStore.setState({ focus: { goalId: "g1", alertId: "a2" } });
    await useWatchStore.getState().ask("g1", "what happened in that run?");
    expect(ipc.watchAsk).toHaveBeenLastCalledWith("g1", "what happened in that run?", "a2");
    expect(useWatchStore.getState().goals[0].thread).toHaveLength(2);
  });

  it("a refresh closes a focused alert whose goal is gone", async () => {
    useWatchStore.setState({ focus: { goalId: "g1", alertId: "a1" } });
    ipc.watchList.mockResolvedValue([]);
    ipc.watchSource.mockResolvedValue({ witness: true, headSeq: 3 });
    await useWatchStore.getState().refresh();
    expect(useWatchStore.getState().focus).toBeNull();
  });
});
