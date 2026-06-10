# Terminal Drag & Drop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dropping files/folders from the OS onto a terminal session types their quoted absolute paths into the agent composer.

**Architecture:** Tauri intercepts webview drag & drop (`dragDropEnabled` defaults to true), so HTML5 drop never fires; instead, each mounted `Terminal` subscribes to `getCurrentWebview().onDragDropEvent`, and on `drop` hit-tests the physical cursor position (÷ `devicePixelRatio`) against its own host rect — only the visible terminal under the cursor writes the quoted paths via the existing `ipc.termWrite`.

**Tech Stack:** `@tauri-apps/api/webview` (already a dependency), xterm.js host element, no Rust changes.

**Spec:** `docs/superpowers/specs/2026-06-10-terminal-drag-drop-design.md`

**Testing note:** No test framework in the repo; verification is `tsc --noEmit` plus the manual checklist in Task 2 (the running `tauri dev` hot-reloads Terminal.tsx).

**Branch:** `feat/clipboard-image-paste` (extends the open PR, as agreed).

---

### Task 1: Drag-drop listener in Terminal.tsx

**Files:**
- Modify: `src/components/Terminal.tsx` (import block ~line 4, listener declarations ~line 71, async IIFE after the `term://exit` listener ~line 86, teardown ~line 210)

- [ ] **Step 1: Add the import**

After `import { listen, type UnlistenFn } from "@tauri-apps/api/event";` add:

```ts
import { getCurrentWebview } from "@tauri-apps/api/webview";
```

- [ ] **Step 2: Declare the unlisten slot**

Next to the existing declarations:

```ts
    let unlistenOutput: UnlistenFn | null = null;
    let unlistenExit: UnlistenFn | null = null;
```

add:

```ts
    let unlistenDragDrop: UnlistenFn | null = null;
```

- [ ] **Step 3: Register the handler inside the async IIFE**

Directly after the `unlistenExit = await listen<TermExit>(...)` block:

```ts
      // Drop files/folders onto this terminal: type their quoted paths into
      // the composer (same flow as the clipboard-image paste below). Tauri
      // intercepts webview drag&drop (dragDropEnabled defaults to true), so
      // HTML5 drop events never fire and this webview event — which carries
      // real OS paths — is the only source. Every mounted Terminal listens;
      // the visibility + hit-test gate picks the one under the cursor.
      unlistenDragDrop = await getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type !== "drop" || !termId) return;
        if (host.style.display === "none") return;
        const scale = window.devicePixelRatio || 1;
        const x = e.payload.position.x / scale;
        const y = e.payload.position.y / scale;
        const r = host.getBoundingClientRect();
        if (x < r.left || x > r.right || y < r.top || y > r.bottom) return;
        const text = e.payload.paths.map((p) => `"${p}"`).join(" ");
        if (text) ipc.termWrite(termId, `${text} `).catch(() => {});
      });
```

- [ ] **Step 4: Dispose in the teardown**

Next to `unlistenOutput?.();` / `unlistenExit?.();` add:

```ts
      unlistenDragDrop?.();
```

- [ ] **Step 5: Typecheck**

Run: `C:\Users\Usuario\work\personal\agent-console\node_modules\.bin\tsc.cmd --noEmit -p tsconfig.json`
Expected: exit 0.

- [ ] **Step 6: Commit**

```powershell
git add src/components/Terminal.tsx
git commit -m "Terminal: type dropped file paths into the agent composer"
```

---

### Task 2: Manual verification (running dev app hot-reloads)

- [ ] Drag one file from Explorer onto the Claude terminal → quoted path appears in the composer.
- [ ] Drag several files at once → all quoted paths, space-separated.
- [ ] Drag a folder → its quoted path appears.
- [ ] Drop over the sidebar (outside the terminal) → nothing happens.
- [ ] Ctrl+V image paste and plain text paste still work.
