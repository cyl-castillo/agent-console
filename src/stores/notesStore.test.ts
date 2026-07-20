import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { StickyNote } from "../types/domain";

const world = vi.hoisted(() => ({
  saved: [] as StickyNote[][],
  listResult: [] as StickyNote[],
  listError: null as string | null,
  saveError: null as string | null,
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    notesList: async () => {
      if (world.listError) throw new Error(world.listError);
      return world.listResult;
    },
    notesSave: async (_root: string, notes: StickyNote[]) => {
      if (world.saveError) throw new Error(world.saveError);
      world.saved.push(notes);
    },
  },
}));

import { NOTE_COLORS, useNotesStore } from "./notesStore";

beforeEach(async () => {
  world.saved = [];
  world.listResult = [];
  world.listError = null;
  world.saveError = null;
  await useNotesStore.getState().load("/repo");
});

afterEach(() => {
  vi.useRealTimers();
});

describe("notes board", () => {
  it("add puts the newest note on top, rotating the palette, and persists", async () => {
    useNotesStore.getState().add();
    useNotesStore.getState().add();
    const notes = useNotesStore.getState().notes;
    expect(notes).toHaveLength(2);
    // Newest first; colors differ between adjacent new notes.
    expect(notes[0].createdAtMs).toBeGreaterThanOrEqual(notes[1].createdAtMs);
    expect(notes[0].color).toBe(NOTE_COLORS[1]);
    expect(notes[1].color).toBe(NOTE_COLORS[0]);
    await new Promise((r) => setTimeout(r, 0));
    expect(world.saved.length).toBe(2);
  });

  it("typing debounces the disk write to one save per burst", async () => {
    vi.useFakeTimers();
    useNotesStore.getState().add();
    await Promise.resolve();
    world.saved = [];

    const id = useNotesStore.getState().notes[0].id;
    useNotesStore.getState().updateText(id, "h");
    useNotesStore.getState().updateText(id, "ho");
    useNotesStore.getState().updateText(id, "hola");
    expect(world.saved.length).toBe(0);

    await vi.advanceTimersByTimeAsync(700);
    expect(world.saved.length).toBe(1);
    expect(world.saved[0][0].text).toBe("hola");
  });

  it("structural changes (color, remove) persist immediately", async () => {
    useNotesStore.getState().add();
    await Promise.resolve();
    world.saved = [];

    const id = useNotesStore.getState().notes[0].id;
    useNotesStore.getState().setColor(id, "pink");
    await new Promise((r) => setTimeout(r, 0));
    expect(world.saved.length).toBe(1);
    expect(world.saved[0][0].color).toBe("pink");

    useNotesStore.getState().remove(id);
    await new Promise((r) => setTimeout(r, 0));
    expect(world.saved.length).toBe(2);
    expect(world.saved[1]).toEqual([]);
  });

  it("load and persist failures surface in error state", async () => {
    world.listError = "unreadable";
    await useNotesStore.getState().load("/repo");
    expect(useNotesStore.getState().error).toContain("unreadable");

    world.listError = null;
    await useNotesStore.getState().load("/repo");
    world.saveError = "disk full";
    await useNotesStore.getState().persist();
    expect(useNotesStore.getState().error).toContain("disk full");
  });

  it("persist without a project is a no-op", async () => {
    useNotesStore.getState().clear();
    await useNotesStore.getState().persist();
    expect(world.saved).toEqual([]);
  });
});
