import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ReflectionResult } from "../types/domain";

// The store talks to the backend and sibling stores; isolate all of it so
// these tests exercise only the learning logic (threshold, cooldown, statuses).
vi.mock("../ipc/tauri", () => ({
  ipc: {
    learningReflect: vi.fn(),
    learningCreateSkill: vi.fn(),
    learningSaveMemory: vi.fn(),
  },
}));
vi.mock("./skillsStore", () => ({
  useSkillsStore: { getState: () => ({ refresh: vi.fn() }) },
}));
vi.mock("./contextStore", () => ({
  useContextStore: { getState: () => ({ refresh: vi.fn() }) },
}));
const toastShow = vi.fn();
vi.mock("./toastStore", () => ({
  useToastStore: { getState: () => ({ show: toastShow }) },
}));

import { ipc } from "../ipc/tauri";
import { useLearningStore } from "./learningStore";

const mockReflect = vi.mocked(ipc.learningReflect);

const REFLECTION: ReflectionResult = {
  suggestions: [
    {
      kind: "skill",
      title: "Deploy helper",
      rationale: "deploys repeat",
      evidence: ["deploy x3"],
      skillName: "deploy-backend",
      skillMdContent: "# deploy",
    },
  ],
  eventsAnalyzed: 42,
  rawExcerpt: "{...}",
};

/// Auto-trigger constants mirrored from learningStore.ts — if those change,
/// these tests should fail loudly rather than silently test the wrong bounds.
const AUTO_THRESHOLD = 15;
const AUTO_COOLDOWN_MS = 10 * 60 * 1000;

const NOW = 1_750_000_000_000;

function resetStore(partial: Partial<ReturnType<typeof useLearningStore.getState>> = {}) {
  useLearningStore.setState({
    status: "idle",
    items: [],
    errorMessage: null,
    rawExcerpt: null,
    eventsAnalyzed: 0,
    autoEnabled: true,
    lastWasAuto: false,
    sinceReflection: 0,
    lastAutoMs: 0,
    ...partial,
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  vi.clearAllMocks();
  resetStore();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("manual reflect", () => {
  it("success: items land as proposed and the counter resets", async () => {
    mockReflect.mockResolvedValue(REFLECTION);
    useLearningStore.setState({ sinceReflection: 7 });

    await useLearningStore.getState().reflect();

    const s = useLearningStore.getState();
    expect(s.status).toBe("results");
    expect(s.items).toHaveLength(1);
    expect(s.items[0].status).toBe("proposed");
    expect(s.eventsAnalyzed).toBe(42);
    expect(s.sinceReflection).toBe(0);
    expect(s.lastWasAuto).toBe(false);
    expect(toastShow).not.toHaveBeenCalled();
  });

  it("failure: surfaces the error to the panel", async () => {
    mockReflect.mockRejectedValue(new Error("claude not found"));

    await useLearningStore.getState().reflect();

    const s = useLearningStore.getState();
    expect(s.status).toBe("error");
    expect(s.errorMessage).toBe("claude not found");
  });
});

describe("auto-trigger via noteActivity", () => {
  function noteActivityTimes(n: number) {
    for (let i = 0; i < n; i++) useLearningStore.getState().noteActivity();
  }

  it("fires only once the prompt threshold is reached", async () => {
    mockReflect.mockResolvedValue(REFLECTION);

    noteActivityTimes(AUTO_THRESHOLD - 1);
    expect(mockReflect).not.toHaveBeenCalled();
    expect(useLearningStore.getState().sinceReflection).toBe(AUTO_THRESHOLD - 1);

    noteActivityTimes(1);
    expect(mockReflect).toHaveBeenCalledTimes(1);

    await vi.waitFor(() => {
      expect(useLearningStore.getState().status).toBe("results");
    });
    const s = useLearningStore.getState();
    expect(s.lastWasAuto).toBe(true);
    expect(s.lastAutoMs).toBe(NOW);
    expect(toastShow).toHaveBeenCalledTimes(1);
  });

  it("respects the cooldown between auto-reflections", () => {
    mockReflect.mockResolvedValue(REFLECTION);
    resetStore({
      sinceReflection: AUTO_THRESHOLD,
      lastAutoMs: NOW - AUTO_COOLDOWN_MS + 1_000, // still inside the window
    });

    useLearningStore.getState().noteActivity();
    expect(mockReflect).not.toHaveBeenCalled();

    // Once the window elapses, the same accumulated activity may fire.
    vi.setSystemTime(NOW + 2_000);
    useLearningStore.getState().noteActivity();
    expect(mockReflect).toHaveBeenCalledTimes(1);
  });

  it("never clobbers a batch the user is still reviewing", () => {
    mockReflect.mockResolvedValue(REFLECTION);
    resetStore({
      sinceReflection: AUTO_THRESHOLD,
      items: [
        {
          ...REFLECTION.suggestions[0],
          id: "x",
          status: "proposed",
        },
      ],
    });

    useLearningStore.getState().noteActivity();
    expect(mockReflect).not.toHaveBeenCalled();
  });

  it("does nothing when auto mode is off", () => {
    resetStore({ autoEnabled: false, sinceReflection: AUTO_THRESHOLD });
    useLearningStore.getState().noteActivity();
    expect(mockReflect).not.toHaveBeenCalled();
    // Counter doesn't even advance — activity tracking is part of auto mode.
    expect(useLearningStore.getState().sinceReflection).toBe(AUTO_THRESHOLD);
  });

  it("a failed auto-reflection stays silent (idle, no error panel)", async () => {
    mockReflect.mockRejectedValue(new Error("boom"));
    resetStore({ sinceReflection: AUTO_THRESHOLD });

    useLearningStore.getState().noteActivity();
    await vi.waitFor(() => {
      expect(useLearningStore.getState().status).toBe("idle");
    });
    expect(useLearningStore.getState().errorMessage).toBeNull();
    expect(toastShow).not.toHaveBeenCalled();
  });
});

describe("apply / skip", () => {
  it("apply materializes a skill and records where it went", async () => {
    mockReflect.mockResolvedValue(REFLECTION);
    await useLearningStore.getState().reflect();
    const id = useLearningStore.getState().items[0].id;
    vi.mocked(ipc.learningCreateSkill).mockResolvedValue("/proj/.claude/skills/deploy-backend/SKILL.md");

    await useLearningStore.getState().apply(id);

    const item = useLearningStore.getState().items[0];
    expect(item.status).toBe("applied");
    expect(item.appliedPath).toContain("deploy-backend");
    expect(ipc.learningCreateSkill).toHaveBeenCalledWith("deploy-backend", "# deploy");
  });

  it("apply failure marks just that item as errored", async () => {
    mockReflect.mockResolvedValue(REFLECTION);
    await useLearningStore.getState().reflect();
    const id = useLearningStore.getState().items[0].id;
    vi.mocked(ipc.learningCreateSkill).mockRejectedValue(new Error("disk full"));

    await useLearningStore.getState().apply(id);

    const item = useLearningStore.getState().items[0];
    expect(item.status).toBe("error");
    expect(item.errorMessage).toBe("disk full");
    expect(useLearningStore.getState().status).toBe("results");
  });

  it("skip marks the item without touching the backend", async () => {
    mockReflect.mockResolvedValue(REFLECTION);
    await useLearningStore.getState().reflect();
    const id = useLearningStore.getState().items[0].id;

    useLearningStore.getState().skip(id);

    expect(useLearningStore.getState().items[0].status).toBe("skipped");
    expect(ipc.learningCreateSkill).not.toHaveBeenCalled();
  });
});
