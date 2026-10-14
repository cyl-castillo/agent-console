import { describe, expect, it } from "vitest";

// Vite `?raw` import (typed by vite/client): the source as a string, no
// node:fs — the frontend tsconfig has no node types on purpose.
import app from "./App.tsx?raw";
import overlays from "./app/OverlayHost.tsx?raw";

// Source-level guard for the two singleton hosts the app must mount in EVERY
// render branch: Toasts and ConfirmDialog. v0.80.0 shipped ConfirmDialog
// mounted twice in the no-project screen and not at all in the main view —
// every `confirmDialog()` there (close session, discard changes, delete
// memory/vault/job…) waited forever on a dialog nobody rendered. Since the
// shell extraction both live in OverlayHost, which App mounts once per
// branch; the check reads both sources.
describe("every App render branch mounts the overlay host", () => {
  it("OverlayHost mounts exactly one Toasts, immediately followed by ConfirmDialog", () => {
    expect(overlays.match(/<Toasts \/>/g)?.length ?? 0).toBe(1);
    expect(overlays.match(/<ConfirmDialog \/>/g)?.length ?? 0).toBe(1);
    expect(overlays.match(/<Toasts \/>\n\s*<ConfirmDialog \/>/g)?.length ?? 0).toBe(1);
  });

  it("App returns OverlayHost from each of its render branches", () => {
    const branches = app.match(/return \(/g)?.length ?? 0;
    const hosts = app.match(/<OverlayHost\b/g)?.length ?? 0;
    expect(branches).toBeGreaterThanOrEqual(2);
    expect(hosts).toBe(branches);
  });
});
