import { beforeEach, describe, expect, it, vi } from "vitest";

// restoreSnapshot is the single most destructive user action in the app: it
// resets the working tree to an earlier turn. Its safety net (UX P0.1) is that
// the backend takes a *pre-restore* backup and returns its sha, which the store
// stashes as `undoRestoreSha` so the restore is itself undoable. These tests
// lock that contract in. We isolate the backend and the two stores restore
// touches (changes refresh + toast).
vi.mock("../ipc/tauri", () => ({
  ipc: {
    snapshotRestore: vi.fn(),
    // referenced elsewhere in the store but not by the paths under test:
    skillList: vi.fn().mockResolvedValue([]),
    hooksStatus: vi.fn(),
    hooksInstall: vi.fn(),
    hooksUninstall: vi.fn(),
    skillRead: vi.fn(),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

const refresh = vi.fn().mockResolvedValue(undefined);
vi.mock("./changesStore", () => ({
  useChangesStore: { getState: () => ({ refresh }) },
}));

const showToast = vi.fn();
vi.mock("./toastStore", () => ({
  useToastStore: { getState: () => ({ show: showToast }) },
}));

import { ipc } from "../ipc/tauri";
import { useSkillsStore } from "./skillsStore";

const mockRestore = vi.mocked(ipc.snapshotRestore);
const store = () => useSkillsStore.getState();

beforeEach(() => {
  vi.clearAllMocks();
  useSkillsStore.setState({ undoRestoreSha: null });
});

describe("restoreSnapshot (UX P0.1 — undoable restore)", () => {
  it("stashes the backend pre-restore sha so the restore can itself be undone", async () => {
    mockRestore.mockResolvedValueOnce("undo-sha-123");

    await store().restoreSnapshot("target-sha");

    expect(mockRestore).toHaveBeenCalledWith("target-sha");
    expect(store().undoRestoreSha).toBe("undo-sha-123");
    expect(refresh).toHaveBeenCalled();
    expect(showToast).toHaveBeenCalledWith(expect.stringMatching(/undo/i), "success");
  });

  it("handles a backend that returns no undo sha without breaking (still success)", async () => {
    mockRestore.mockResolvedValueOnce(null as unknown as string);

    await store().restoreSnapshot("target-sha");

    expect(store().undoRestoreSha).toBeNull();
    expect(showToast).toHaveBeenCalledWith("Restored", "success");
  });

  it("on failure shows an error toast, does not throw, and leaves the prior undo sha intact", async () => {
    useSkillsStore.setState({ undoRestoreSha: "previous-undo" });
    mockRestore.mockRejectedValueOnce(new Error("bad object"));

    await expect(store().restoreSnapshot("target-sha")).resolves.toBeUndefined();

    // A failed restore must not clobber the existing undo pointer or refresh.
    expect(store().undoRestoreSha).toBe("previous-undo");
    expect(refresh).not.toHaveBeenCalled();
    expect(showToast).toHaveBeenCalledWith(expect.stringContaining("Restore failed"), "error");
  });
});
