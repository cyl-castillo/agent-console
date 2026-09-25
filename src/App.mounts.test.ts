import { describe, expect, it } from "vitest";

// Vite `?raw` import (typed by vite/client): the source as a string, no
// node:fs — the frontend tsconfig has no node types on purpose.
import src from "./App.tsx?raw";

// Source-level guard for the two singleton hosts App must mount in EVERY
// render branch: Toasts and ConfirmDialog. v0.80.0 shipped ConfirmDialog
// mounted twice in the no-project screen and not at all in the main view —
// every `confirmDialog()` there (close session, discard changes, delete
// memory/vault/job…) waited forever on a dialog nobody rendered. There is
// no component test harness here, so the check reads the source: each
// `<Toasts />` must be immediately followed by `<ConfirmDialog />`.
describe("App mounts the confirm host next to every Toasts host", () => {
  it("has the same number of Toasts and ConfirmDialog mounts, and at least two", () => {
    const toasts = src.match(/<Toasts \/>/g)?.length ?? 0;
    const confirms = src.match(/<ConfirmDialog \/>/g)?.length ?? 0;
    expect(toasts).toBeGreaterThanOrEqual(2);
    expect(confirms).toBe(toasts);
  });

  it("pairs them: every Toasts is followed by a ConfirmDialog on the next line", () => {
    const pairs = src.match(/<Toasts \/>\n\s*<ConfirmDialog \/>/g)?.length ?? 0;
    const toasts = src.match(/<Toasts \/>/g)?.length ?? 0;
    expect(pairs).toBe(toasts);
  });
});
