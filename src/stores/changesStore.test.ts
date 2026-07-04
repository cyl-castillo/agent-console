import { beforeEach, describe, expect, it, vi } from "vitest";

import type { GitFileChange, GitStatus } from "../types/domain";

// Isolate the backend, the tauri event bridge, and the scheduler side-effect so
// these tests exercise only the store's own logic. The focus is the destructive
// surface (revert / revertAll / checkoutBranch) plus the silent-correctness
// paths (stale-diff race guard, selection after refresh, commit gating).
vi.mock("../ipc/tauri", () => ({
  ipc: {
    gitStatus: vi.fn(),
    gitDiffFile: vi.fn(),
    gitRevertFile: vi.fn(),
    gitStageFile: vi.fn(),
    gitUnstageFile: vi.fn(),
    gitCommit: vi.fn(),
    gitAmendCommit: vi.fn(),
    gitRecentMessages: vi.fn(),
    gitHeadMessage: vi.fn(),
    gitBranches: vi.fn(),
    gitCheckoutBranch: vi.fn(),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("./schedulerStore", () => ({
  fireSchedulerEvent: vi.fn(),
}));

import { ipc } from "../ipc/tauri";
import { fireSchedulerEvent } from "./schedulerStore";
import { useChangesStore } from "./changesStore";

const mockStatus = vi.mocked(ipc.gitStatus);
const mockDiff = vi.mocked(ipc.gitDiffFile);
const mockRevert = vi.mocked(ipc.gitRevertFile);
const mockCommit = vi.mocked(ipc.gitCommit);
const mockAmend = vi.mocked(ipc.gitAmendCommit);
const mockBranches = vi.mocked(ipc.gitBranches);
const mockCheckout = vi.mocked(ipc.gitCheckoutBranch);
const mockFire = vi.mocked(fireSchedulerEvent);

function change(over: Partial<GitFileChange> = {}): GitFileChange {
  return {
    path: "src/a.ts",
    code: "M",
    staged: false,
    unstaged: true,
    untracked: false,
    ...over,
  };
}

function status(changes: GitFileChange[]): GitStatus {
  return { isRepo: true, branch: "main", changes };
}

const store = () => useChangesStore.getState();

beforeEach(() => {
  vi.clearAllMocks();
  // A clean baseline; individual tests seed `status` via setState where needed.
  useChangesStore.setState({
    status: null,
    selected: null,
    diff: "",
    loading: false,
    error: null,
    commitMessage: "",
    committing: false,
    recentMessages: [],
    branches: [],
    branchesLoading: false,
  });
  // Default: refresh() re-reads an empty tree unless a test overrides it.
  mockStatus.mockResolvedValue(status([]));
  mockDiff.mockResolvedValue("");
  mockRevert.mockResolvedValue(undefined);
  mockBranches.mockResolvedValue([]);
});

describe("revertAll (mass discard — UX P0.3)", () => {
  it("reverts every change, iterating all paths", async () => {
    const changes = [
      change({ path: "a.ts", untracked: false }),
      change({ path: "b.ts", untracked: true, code: "U" }), // a new file (rm on backend)
      change({ path: "c.ts", untracked: false }),
    ];
    useChangesStore.setState({ status: status(changes) });

    await store().revertAll();

    expect(mockRevert).toHaveBeenCalledTimes(3);
    expect(mockRevert).toHaveBeenCalledWith("a.ts");
    expect(mockRevert).toHaveBeenCalledWith("b.ts");
    expect(mockRevert).toHaveBeenCalledWith("c.ts");
  });

  it("keeps going when one revert throws, and still refreshes at the end", async () => {
    useChangesStore.setState({
      status: status([change({ path: "a.ts" }), change({ path: "b.ts" }), change({ path: "c.ts" })]),
    });
    mockRevert.mockRejectedValueOnce(new Error("locked")); // a.ts fails

    await store().revertAll();

    // The batch must not abort on the first failure.
    expect(mockRevert).toHaveBeenCalledTimes(3);
    // refresh() runs afterwards to re-read the (partially reverted) tree.
    expect(mockStatus).toHaveBeenCalled();
  });

  it("no-ops when there is no status (nothing to revert)", async () => {
    useChangesStore.setState({ status: null });
    await store().revertAll();
    expect(mockRevert).not.toHaveBeenCalled();
    expect(mockStatus).not.toHaveBeenCalled();
  });
});

describe("revert (single file)", () => {
  it("reverts then refreshes", async () => {
    await store().revert("a.ts");
    expect(mockRevert).toHaveBeenCalledWith("a.ts");
    expect(mockStatus).toHaveBeenCalled();
  });

  it("records the error instead of throwing when the backend fails", async () => {
    mockRevert.mockRejectedValueOnce(new Error("boom"));
    await expect(store().revert("a.ts")).resolves.toBeUndefined();
    expect(store().error).toContain("boom");
  });
});

describe("refresh selection", () => {
  it("keeps the previously selected file when it still exists", async () => {
    useChangesStore.setState({ selected: "b.ts" });
    mockStatus.mockResolvedValueOnce(
      status([change({ path: "a.ts" }), change({ path: "b.ts" })]),
    );
    await store().refresh();
    expect(store().selected).toBe("b.ts");
  });

  it("falls back to the first change when the selection is gone", async () => {
    useChangesStore.setState({ selected: "gone.ts" });
    mockStatus.mockResolvedValueOnce(status([change({ path: "a.ts" })]));
    await store().refresh();
    expect(store().selected).toBe("a.ts");
  });

  it("clears selection and diff when the tree is clean", async () => {
    useChangesStore.setState({ selected: "a.ts", diff: "old" });
    mockStatus.mockResolvedValueOnce(status([]));
    await store().refresh();
    expect(store().selected).toBeNull();
    expect(store().diff).toBe("");
  });

  it("records the error and stops loading when gitStatus fails", async () => {
    mockStatus.mockRejectedValueOnce(new Error("not a repo"));
    await store().refresh();
    expect(store().error).toContain("not a repo");
    expect(store().loading).toBe(false);
  });
});

describe("setSelected stale-diff race guard", () => {
  it("does not overwrite the diff with a stale result when selection moved on", async () => {
    let resolveA: (v: string) => void = () => {};
    mockDiff.mockImplementation((f: string) =>
      f === "a.ts"
        ? new Promise<string>((r) => { resolveA = r; })
        : Promise.resolve("diff-B"),
    );

    const pendingA = store().setSelected("a.ts"); // selects a.ts, diff still in flight
    await store().setSelected("b.ts"); // selection moves to b.ts, diff-B applied

    resolveA("diff-A"); // the slow a.ts diff finally resolves
    await pendingA;

    // The late a.ts diff must be discarded — we're on b.ts now.
    expect(store().selected).toBe("b.ts");
    expect(store().diff).toBe("diff-B");
  });
});

describe("checkoutBranch (destructive — UX P1.11)", () => {
  it("checks out then refreshes and reloads branches", async () => {
    mockCheckout.mockResolvedValueOnce(undefined);
    await store().checkoutBranch("feature");
    expect(mockCheckout).toHaveBeenCalledWith("feature");
    expect(mockStatus).toHaveBeenCalled();
    expect(mockBranches).toHaveBeenCalled();
  });

  it("re-throws on failure so the caller can surface it, and records the error", async () => {
    mockCheckout.mockRejectedValueOnce(new Error("dirty tree"));
    await expect(store().checkoutBranch("feature")).rejects.toThrow("dirty tree");
    expect(store().error).toContain("dirty tree");
  });
});

describe("commit", () => {
  it("returns null without touching the backend when the message is blank", async () => {
    useChangesStore.setState({ commitMessage: "   " });
    const sha = await store().commit();
    expect(sha).toBeNull();
    expect(mockCommit).not.toHaveBeenCalled();
  });

  it("commits, clears the message, fires the scheduler commit event, and returns the sha", async () => {
    useChangesStore.setState({ commitMessage: "feat: thing" });
    mockCommit.mockResolvedValueOnce("abc123");
    const sha = await store().commit();
    expect(sha).toBe("abc123");
    expect(mockCommit).toHaveBeenCalledWith("feat: thing");
    expect(store().commitMessage).toBe("");
    expect(mockFire).toHaveBeenCalledWith("commit");
  });

  it("amends when asked", async () => {
    useChangesStore.setState({ commitMessage: "reword" });
    mockAmend.mockResolvedValueOnce("def456");
    const sha = await store().commit({ amend: true });
    expect(sha).toBe("def456");
    expect(mockAmend).toHaveBeenCalledWith("reword");
    expect(mockCommit).not.toHaveBeenCalled();
  });

  it("keeps the message and records the error when the commit fails", async () => {
    useChangesStore.setState({ commitMessage: "feat: thing" });
    mockCommit.mockRejectedValueOnce(new Error("hook rejected"));
    const sha = await store().commit();
    expect(sha).toBeNull();
    expect(store().commitMessage).toBe("feat: thing");
    expect(store().error).toContain("hook rejected");
    expect(store().committing).toBe(false);
  });
});
