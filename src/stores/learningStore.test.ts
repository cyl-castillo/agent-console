import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ReflectionResult } from "../types/domain";

// The store talks to the backend and sibling stores; isolate all of it so
// these tests exercise only the learning logic (threshold, cooldown, statuses).
vi.mock("../ipc/tauri", () => ({
  ipc: {
    learningReflect: vi.fn(),
    learningCreateSkill: vi.fn(),
    learningCreatePlugin: vi.fn(),
    learningSaveMemory: vi.fn(),
    learningCurate: vi.fn(),
    learningApplyMerge: vi.fn(),
    learningApplyRefactor: vi.fn(),
    learningApplyArchive: vi.fn(),
  },
}));
// Mutable corpus inventory the curation auto-trigger reads from. Tests set these
// to size the corpus, then call noteCorpusSize().
const corpus = { skills: [] as unknown[], memories: [] as unknown[] };
function setCorpus(skills: number, memories: number) {
  corpus.skills = Array.from({ length: skills }, () => ({ source: "project", kind: "skill" }));
  corpus.memories = Array.from({ length: memories }, () => ({ isIndex: false }));
}
vi.mock("./skillsStore", () => ({
  useSkillsStore: { getState: () => ({ refresh: vi.fn(), installed: corpus.skills }) },
}));
vi.mock("./contextStore", () => ({
  useContextStore: { getState: () => ({ refresh: vi.fn(), memories: corpus.memories }) },
}));
const toastShow = vi.fn();
vi.mock("./toastStore", () => ({
  useToastStore: { getState: () => ({ show: toastShow }) },
}));

import { ipc } from "../ipc/tauri";
import { useLearningStore } from "./learningStore";

const mockReflect = vi.mocked(ipc.learningReflect);
const mockCurate = vi.mocked(ipc.learningCurate);

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
const CURATE_MIN_CORPUS = 8;
const CURATE_GROWTH = 5;
const CURATE_COOLDOWN_MS = 30 * 60 * 1000;

const NOW = 1_750_000_000_000;

const CURATION = {
  suggestions: [
    {
      action: "merge" as const,
      targetKind: "memory" as const,
      targets: ["a.md", "b.md"],
      title: "Fuse overlapping deploy notes",
      rationale: "two memories say the same thing",
      evidence: ["both mention lightsail"],
      newName: "deploy.md",
      newContent: "# deploy",
    },
  ],
  skillsAnalyzed: 6,
  memoriesAnalyzed: 4,
  rawExcerpt: "{...}",
};

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
    curationStatus: "idle",
    curationItems: [],
    curationError: null,
    skillsAnalyzed: 0,
    memoriesAnalyzed: 0,
    curateAutoEnabled: true,
    lastCuratedSize: 0,
    lastCurateMs: 0,
    ...partial,
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  vi.clearAllMocks();
  setCorpus(0, 0);
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
    vi.mocked(ipc.learningCreateSkill).mockResolvedValue(
      "/proj/.claude/skills/deploy-backend/SKILL.md",
    );

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

  it("apply scaffolds a plugin suggestion via learningCreatePlugin", async () => {
    mockReflect.mockResolvedValue({
      ...REFLECTION,
      suggestions: [
        {
          kind: "plugin",
          title: "Package release helpers",
          rationale: "used across repos",
          evidence: ["release flow in 3 projects"],
          pluginName: "release-helpers",
          pluginDescription: "Release workflow helpers",
          pluginSkillMd: "---\nname: release-helpers\n---\n\nBody",
        },
      ],
    });
    await useLearningStore.getState().reflect();
    const id = useLearningStore.getState().items[0].id;
    vi.mocked(ipc.learningCreatePlugin).mockResolvedValue("/home/u/.claude/skills/release-helpers");

    await useLearningStore.getState().apply(id);

    const item = useLearningStore.getState().items[0];
    expect(item.status).toBe("applied");
    expect(item.appliedPath).toContain("release-helpers");
    expect(ipc.learningCreatePlugin).toHaveBeenCalledWith(
      "release-helpers",
      "Release workflow helpers",
      "---\nname: release-helpers\n---\n\nBody",
    );
  });

  it("hook suggestions are report-only: apply is a no-op", async () => {
    mockReflect.mockResolvedValue({
      ...REFLECTION,
      suggestions: [
        {
          kind: "hook",
          title: "Enforce no direct pushes",
          rationale: "user repeats this rule",
          evidence: ["'no pushees a main' x4"],
        },
      ],
    });
    await useLearningStore.getState().reflect();
    const id = useLearningStore.getState().items[0].id;

    await useLearningStore.getState().apply(id);

    expect(useLearningStore.getState().items[0].status).toBe("proposed");
    expect(ipc.learningCreatePlugin).not.toHaveBeenCalled();
    expect(ipc.learningCreateSkill).not.toHaveBeenCalled();
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

describe("curation auto-trigger via noteCorpusSize", () => {
  it("fires only once the corpus crosses floor + growth", async () => {
    mockCurate.mockResolvedValue(CURATION);

    // Just below the floor → never fires, even with plenty of growth.
    setCorpus(CURATE_MIN_CORPUS - 1, 0);
    useLearningStore.getState().noteCorpusSize();
    expect(mockCurate).not.toHaveBeenCalled();

    // Past the floor, with growth from baseline 0 well over the threshold.
    setCorpus(6, 4); // size 10 >= floor, growth 10 >= CURATE_GROWTH
    expect(10).toBeGreaterThanOrEqual(CURATE_MIN_CORPUS);
    expect(10).toBeGreaterThanOrEqual(CURATE_GROWTH);
    useLearningStore.getState().noteCorpusSize();
    expect(mockCurate).toHaveBeenCalledTimes(1);

    await vi.waitFor(() => {
      expect(useLearningStore.getState().curationStatus).toBe("results");
    });
    // Baseline re-stamped to the analyzed corpus size.
    expect(useLearningStore.getState().lastCuratedSize).toBe(10);
  });

  it("waits for enough new growth since the last pass", () => {
    mockCurate.mockResolvedValue(CURATION);
    resetStore({ lastCuratedSize: 10, lastCurateMs: NOW - CURATE_COOLDOWN_MS - 1 });

    setCorpus(8, 5); // size 13, growth 3 < CURATE_GROWTH
    useLearningStore.getState().noteCorpusSize();
    expect(mockCurate).not.toHaveBeenCalled();

    setCorpus(10, 5); // size 15, growth 5 >= threshold
    useLearningStore.getState().noteCorpusSize();
    expect(mockCurate).toHaveBeenCalledTimes(1);
  });

  it("respects the cooldown between auto-curations", () => {
    mockCurate.mockResolvedValue(CURATION);
    resetStore({ lastCuratedSize: 0, lastCurateMs: NOW - CURATE_COOLDOWN_MS + 1_000 });
    setCorpus(8, 5); // size 13, well past floor + growth

    useLearningStore.getState().noteCorpusSize();
    expect(mockCurate).not.toHaveBeenCalled();

    vi.setSystemTime(NOW + 2_000); // cooldown window elapsed
    useLearningStore.getState().noteCorpusSize();
    expect(mockCurate).toHaveBeenCalledTimes(1);
  });

  it("does nothing when auto-curate is off", () => {
    resetStore({ curateAutoEnabled: false });
    setCorpus(20, 20);
    useLearningStore.getState().noteCorpusSize();
    expect(mockCurate).not.toHaveBeenCalled();
  });

  it("never clobbers curation results the user is still reviewing", () => {
    mockCurate.mockResolvedValue(CURATION);
    resetStore({
      curationItems: [{ ...CURATION.suggestions[0], id: "x", status: "proposed" }],
    });
    setCorpus(10, 5);
    useLearningStore.getState().noteCorpusSize();
    expect(mockCurate).not.toHaveBeenCalled();
  });

  it("applyCuration(merge) calls the merge IPC and marks applied", async () => {
    mockCurate.mockResolvedValue(CURATION);
    await useLearningStore.getState().curate();
    const id = useLearningStore.getState().curationItems[0].id;
    vi.mocked(ipc.learningApplyMerge).mockResolvedValue("/proj/memory/deploy.md");

    await useLearningStore.getState().applyCuration(id);

    const item = useLearningStore.getState().curationItems[0];
    expect(item.status).toBe("applied");
    expect(ipc.learningApplyMerge).toHaveBeenCalledWith(
      "memory",
      ["a.md", "b.md"],
      "deploy.md",
      "# deploy",
    );
  });
});
