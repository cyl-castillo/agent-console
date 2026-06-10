# Drag & drop files onto the terminal — design

**Date:** 2026-06-10
**Status:** Approved

## Goal

Dragging files or folders from the OS onto a terminal session types their
quoted absolute paths into the agent composer — the complement of the
clipboard-image paste feature (see
`2026-06-09-clipboard-image-paste-design.md`).

## Background

Tauri's `dragDropEnabled` is on by default (not overridden in
`tauri.conf.json`), so the webview's HTML5 drag & drop is intercepted by
Tauri and dropping a file currently does nothing. Tauri instead emits its
own webview drag-drop events carrying the real OS paths and the cursor
position — HTML5 `File` objects never expose real paths, so the Tauri
event is the only viable source (HTML5 approach rejected).

## Design

All in `src/components/Terminal.tsx`, registered inside the existing mount
effect's async IIFE (same pattern as the `term://output` listeners):

- `getCurrentWebview().onDragDropEvent(cb)` from `@tauri-apps/api/webview`;
  keep the `UnlistenFn` and dispose in the teardown.
- On `payload.type === "drop"`:
  - no-op if `termId` is null or this terminal's host is hidden
    (`display: none`);
  - hit-test: `payload.position` is in physical pixels — divide by
    `window.devicePixelRatio` and compare against the host's
    `getBoundingClientRect()`; no-op when the drop lands outside this
    terminal;
  - else `termWrite` all paths quoted and space-separated, with a trailing
    space: `"C:\a.png" "C:\b.txt" `. Files and folders alike, no type
    filtering.
- Every mounted Terminal registers its own listener; the visibility +
  hit-test gate guarantees only the terminal under the cursor writes.
- Failures are silent (`.catch(() => {})`), consistent with the file.
- No drag-over visual feedback in v1.

Alternative considered and rejected: a single global handler in `App.tsx`
targeting "the active session" via `ac:term-input` — duplicates the
which-terminal-is-visible logic that the per-component hit-test already
answers, and couples App to terminal internals.

## Verification (manual)

1. Drag a file from Explorer onto a Claude session → its quoted path
   appears in the composer.
2. Drag multiple files at once → all quoted paths, space-separated.
3. Drag a folder → its quoted path appears.
4. Drop outside the terminal area (e.g. over the sidebar) → nothing.
5. Clipboard image paste and text paste still work unchanged.

## Out of scope

- Drag-over visual feedback (highlight) — revisit if missed.
- Dropping raw text/URLs (Tauri's event only carries file paths).
