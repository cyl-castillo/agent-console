import { beforeEach, describe, expect, it, vi } from "vitest";

const world = vi.hoisted(() => ({
  hits: [] as { id: string; kind: string; title: string; snippet: string; score: number }[],
  searchError: null as string | null,
  reindexResult: { indexed: 2, reused: 3, removed: 0, total: 5 },
  queries: [] as string[],
  projectRoot: "/repo" as string | null,
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    contextStatus: async () => ({}),
    memoryList: async () => [],
    semanticSearch: async (_root: string, query: string) => {
      if (world.searchError) throw new Error(world.searchError);
      world.queries.push(query);
      return world.hits;
    },
    semanticReindex: async () => world.reindexResult,
  },
}));
vi.mock("./sessionStore", () => ({
  useSessionStore: {
    getState: () => ({ project: world.projectRoot ? { root: world.projectRoot } : null }),
  },
}));
vi.mock("./learningStore", () => ({
  useLearningStore: { getState: () => ({ noteActivity: () => {}, noteCorpusSize: () => {} }) },
}));
vi.mock("./schedulerStore", () => ({ fireSchedulerEvent: async () => {} }));

import { useContextStore } from "./contextStore";

beforeEach(() => {
  world.hits = [];
  world.searchError = null;
  world.queries = [];
  world.projectRoot = "/repo";
  useContextStore.setState({
    searchHits: [],
    searching: false,
    searchError: null,
    reindexing: false,
  });
});

describe("semantic search", () => {
  it("searches with the trimmed query and stores hits", async () => {
    world.hits = [{ id: "memory:a.md", kind: "memory", title: "A", snippet: "…", score: 0.9 }];
    await useContextStore.getState().semanticSearch("  race de guardado  ");
    expect(world.queries).toEqual(["race de guardado"]);
    expect(useContextStore.getState().searchHits).toHaveLength(1);
    expect(useContextStore.getState().searching).toBe(false);
  });

  it("empty query or no project: quiet no-op", async () => {
    await useContextStore.getState().semanticSearch("   ");
    world.projectRoot = null;
    await useContextStore.getState().semanticSearch("algo");
    expect(world.queries).toEqual([]);
  });

  it("failures land in searchError, not a stuck spinner", async () => {
    world.searchError = "model init failed";
    await useContextStore.getState().semanticSearch("q");
    const s = useContextStore.getState();
    expect(s.searching).toBe(false);
    expect(s.searchError).toContain("model init failed");
  });

  it("reindex reports a human summary", async () => {
    const summary = await useContextStore.getState().semanticReindex();
    expect(summary).toBe("5 indexed (2 new, 3 unchanged)");
    expect(useContextStore.getState().reindexing).toBe(false);
  });

  it("clearSearch resets hits and error", async () => {
    useContextStore.setState({
      searchHits: [{ id: "x", kind: "memory", title: "t", snippet: "", score: 1 }],
      searchError: "e",
    });
    useContextStore.getState().clearSearch();
    expect(useContextStore.getState().searchHits).toEqual([]);
    expect(useContextStore.getState().searchError).toBeNull();
  });
});
