import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useAgentStatusStore, WORKING_WINDOW_MS } from "./agentStatusStore";

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1_000_000);
  useAgentStatusStore.setState({ workingUntil: 0, workingSince: 0 });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("agent status pill", () => {
  it("markActive opens a working window anchored at the first activity", () => {
    useAgentStatusStore.getState().markActive();
    const s = useAgentStatusStore.getState();
    expect(s.workingSince).toBe(1_000_000);
    expect(s.workingUntil).toBe(1_000_000 + WORKING_WINDOW_MS);
  });

  it("activity within the window extends it but keeps the original start (elapsed readout)", () => {
    useAgentStatusStore.getState().markActive();
    vi.setSystemTime(1_000_000 + 5_000);
    useAgentStatusStore.getState().markActive();
    const s = useAgentStatusStore.getState();
    expect(s.workingSince).toBe(1_000_000);
    expect(s.workingUntil).toBe(1_005_000 + WORKING_WINDOW_MS);
  });

  it("activity after the window expired starts a new stretch", () => {
    useAgentStatusStore.getState().markActive();
    vi.setSystemTime(1_000_000 + WORKING_WINDOW_MS + 1);
    useAgentStatusStore.getState().markActive();
    expect(useAgentStatusStore.getState().workingSince).toBe(1_000_000 + WORKING_WINDOW_MS + 1);
  });

  it("markIdle (Stop hook) drops to idle immediately instead of waiting out the decay", () => {
    useAgentStatusStore.getState().markActive();
    useAgentStatusStore.getState().markIdle();
    const s = useAgentStatusStore.getState();
    expect(s.workingUntil).toBe(0);
    expect(s.workingSince).toBe(0);
  });
});
