import { describe, expect, it, vi } from "vitest";
import { ACTION_IDS } from "./ids";
// Vite `?raw` import (typed by vite/client): the doc as a string, no node:fs.
import doc from "../../docs/ACTIONS.md?raw";

// The palette registry drags the theme/voice stores along; they are
// node-safe now, and the executor is not needed to read ids.
vi.mock("../ipc/tauri", () => ({ ipc: {} }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

describe("action ids are a stable, documented contract", () => {
  it("the palette registry and ACTION_IDS are the same set", async () => {
    const { allPaletteActionIds } = await import("../stores/paletteStore");
    const registry = [...allPaletteActionIds()].sort();
    expect(registry).toEqual([...ACTION_IDS].sort());
  });
  it("every id is documented in docs/ACTIONS.md", () => {
    const missing = ACTION_IDS.filter((id) => !doc.includes(`\`${id}\``));
    expect(missing).toEqual([]);
  });
});
