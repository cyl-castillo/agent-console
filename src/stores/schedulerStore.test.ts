import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Job, RunRecord } from "../types/domain";

// Isolate the backend and the tauri event bridge so these tests exercise only
// the store's own logic (optimistic toggle, event handling, history merge).
vi.mock("../ipc/tauri", () => ({
  ipc: {
    schedulerList: vi.fn(),
    schedulerCreate: vi.fn(),
    schedulerUpdate: vi.fn(),
    schedulerDelete: vi.fn(),
    schedulerSetEnabled: vi.fn(),
    schedulerHistory: vi.fn(),
    schedulerFireEvent: vi.fn(),
    schedulerIsPaused: vi.fn().mockResolvedValue(false),
    schedulerSetPaused: vi.fn(),
    schedulerRunNow: vi.fn(),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { ipc } from "../ipc/tauri";
import { fireSchedulerEvent, SCHEDULER_EVENTS, useSchedulerStore } from "./schedulerStore";

const mockList = vi.mocked(ipc.schedulerList);
const mockHistory = vi.mocked(ipc.schedulerHistory);
const mockCreate = vi.mocked(ipc.schedulerCreate);
const mockDelete = vi.mocked(ipc.schedulerDelete);
const mockSetEnabled = vi.mocked(ipc.schedulerSetEnabled);
const mockRunNow = vi.mocked(ipc.schedulerRunNow);
const mockFireEvent = vi.mocked(ipc.schedulerFireEvent);
const mockIsPaused = vi.mocked(ipc.schedulerIsPaused);
const mockSetPaused = vi.mocked(ipc.schedulerSetPaused);

function job(over: Partial<Job> = {}): Job {
  return {
    id: "j1",
    name: "nightly",
    enabled: true,
    trigger: { type: "interval", everyMs: 60_000 },
    action: { type: "prompt", text: "hi" },
    onMissed: "catchup",
    cooldownMs: 0,
    createdAtMs: 1,
    nextDueMs: 100,
    consecutiveFailures: 0,
    ...over,
  };
}

function rec(over: Partial<RunRecord> = {}): RunRecord {
  return {
    jobId: "j1",
    jobName: "nightly",
    startedMs: 10,
    finishedMs: 20,
    status: "ok",
    summary: "did it",
    outputExcerpt: "out",
    ...over,
  };
}

function reset() {
  useSchedulerStore.setState({
    jobs: [],
    history: [],
    runningJobIds: [],
    paused: false,
    status: "idle",
    errorMessage: null,
  });
  vi.clearAllMocks();
  mockIsPaused.mockResolvedValue(false);
}

describe("schedulerStore", () => {
  beforeEach(reset);

  it("refresh loads jobs and history together", async () => {
    mockList.mockResolvedValue([job()]);
    mockHistory.mockResolvedValue([rec()]);
    await useSchedulerStore.getState().refresh();
    const s = useSchedulerStore.getState();
    expect(s.status).toBe("ready");
    expect(s.jobs).toHaveLength(1);
    expect(s.history).toHaveLength(1);
  });

  it("refresh surfaces an error without throwing", async () => {
    mockList.mockRejectedValue(new Error("no project open"));
    mockHistory.mockResolvedValue([]);
    await useSchedulerStore.getState().refresh();
    const s = useSchedulerStore.getState();
    expect(s.status).toBe("error");
    expect(s.errorMessage).toContain("no project");
  });

  it("createJob calls the backend then refreshes", async () => {
    mockCreate.mockResolvedValue(job());
    mockList.mockResolvedValue([job()]);
    mockHistory.mockResolvedValue([]);
    await useSchedulerStore.getState().createJob(job({ id: "" }));
    expect(mockCreate).toHaveBeenCalledOnce();
    expect(useSchedulerStore.getState().jobs).toHaveLength(1);
  });

  it("setEnabled flips optimistically and reconciles from the result", async () => {
    useSchedulerStore.setState({ jobs: [job({ enabled: true })] });
    // Resolve slowly so we can observe the optimistic state first.
    let resolveSet: (j: Job) => void = () => {};
    mockSetEnabled.mockImplementation(
      () => new Promise<Job>((res) => { resolveSet = res; }),
    );
    const p = useSchedulerStore.getState().setEnabled("j1", false);
    // Optimistic: already shows disabled before the IPC resolves.
    expect(useSchedulerStore.getState().jobs[0].enabled).toBe(false);
    resolveSet(job({ enabled: false, nextDueMs: undefined }));
    await p;
    expect(useSchedulerStore.getState().jobs[0].enabled).toBe(false);
  });

  it("setEnabled reverts to server truth on failure", async () => {
    useSchedulerStore.setState({ jobs: [job({ enabled: true })] });
    mockSetEnabled.mockRejectedValue(new Error("boom"));
    mockList.mockResolvedValue([job({ enabled: true })]); // server still enabled
    mockHistory.mockResolvedValue([]);
    await expect(useSchedulerStore.getState().setEnabled("j1", false)).rejects.toThrow();
    expect(useSchedulerStore.getState().jobs[0].enabled).toBe(true);
  });

  it("runNow invokes the backend and refreshes", async () => {
    mockRunNow.mockResolvedValue(rec());
    mockList.mockResolvedValue([job()]);
    mockHistory.mockResolvedValue([rec()]);
    await useSchedulerStore.getState().runNow("j1");
    expect(mockRunNow).toHaveBeenCalledWith("j1");
  });

  it("deleteJob calls the backend then refreshes", async () => {
    mockDelete.mockResolvedValue(undefined);
    mockList.mockResolvedValue([]);
    mockHistory.mockResolvedValue([]);
    useSchedulerStore.setState({ jobs: [job()] });
    await useSchedulerStore.getState().deleteJob("j1");
    expect(mockDelete).toHaveBeenCalledWith("j1");
    expect(useSchedulerStore.getState().jobs).toHaveLength(0);
  });

  it("run_started marks a job running; run_finished clears it and prepends history", () => {
    mockList.mockResolvedValue([job()]);
    mockHistory.mockResolvedValue([]);
    const s = useSchedulerStore.getState();
    s._onRunStarted("j1");
    expect(useSchedulerStore.getState().runningJobIds).toContain("j1");
    // Duplicate start is a no-op.
    s._onRunStarted("j1");
    expect(useSchedulerStore.getState().runningJobIds).toHaveLength(1);

    s._onRunFinished(rec({ summary: "fresh" }));
    const after = useSchedulerStore.getState();
    expect(after.runningJobIds).not.toContain("j1");
    expect(after.history[0].summary).toBe("fresh");
  });

  it("refresh reflects the global paused flag", async () => {
    mockList.mockResolvedValue([]);
    mockHistory.mockResolvedValue([]);
    mockIsPaused.mockResolvedValue(true);
    await useSchedulerStore.getState().refresh();
    expect(useSchedulerStore.getState().paused).toBe(true);
  });

  it("setPaused flips optimistically and reverts on failure", async () => {
    mockSetPaused.mockResolvedValue(undefined);
    await useSchedulerStore.getState().setPaused(true);
    expect(mockSetPaused).toHaveBeenCalledWith(true);
    expect(useSchedulerStore.getState().paused).toBe(true);

    mockSetPaused.mockRejectedValue(new Error("boom"));
    await expect(useSchedulerStore.getState().setPaused(false)).rejects.toThrow();
    expect(useSchedulerStore.getState().paused).toBe(true); // reverted
  });

  it("paused_changed event updates state", () => {
    useSchedulerStore.getState()._onPausedChanged(true);
    expect(useSchedulerStore.getState().paused).toBe(true);
  });

  it("fireSchedulerEvent forwards the name and swallows failures", async () => {
    mockFireEvent.mockResolvedValue(undefined);
    await fireSchedulerEvent("commit");
    expect(mockFireEvent).toHaveBeenCalledWith("commit");

    // A rejected call (no project open) must not throw into the hot path.
    mockFireEvent.mockRejectedValue(new Error("no project open"));
    await expect(fireSchedulerEvent("prompt")).resolves.toBeUndefined();
  });

  it("exposes a curated event catalog the editor offers", () => {
    expect(SCHEDULER_EVENTS.map((e) => e.name)).toContain("corpus_grew");
    expect(SCHEDULER_EVENTS.length).toBeGreaterThanOrEqual(3);
  });
});
