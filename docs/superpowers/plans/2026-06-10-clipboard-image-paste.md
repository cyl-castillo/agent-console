# Clipboard Image Paste Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the clipboard holds an image and no text, pressing Ctrl+V (or Cmd+V / context-menu Paste) in a terminal session forwards the Ctrl+V byte (`0x16`) to the PTY so the agent CLI (Claude Code, Codex) attaches the image with its own native clipboard support.

**Architecture:** Frontend-only change. A capture-phase `paste` listener on the xterm host element in `Terminal.tsx` inspects the synchronous `clipboardData`: text pastes fall through to xterm unchanged; image-only pastes are swallowed and replaced by `ipc.termWrite(termId, "\x16")`. No Rust changes, no new IPC, no per-agent code. One new row in `ShortcutsModal.tsx` for discoverability.

**Tech Stack:** React 19 + TypeScript, xterm.js 6, existing Tauri 2 `term_write` IPC. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-06-09-clipboard-image-paste-design.md`

**Testing note:** The repo has no test framework (`package.json` has no `test` script), and the behavior is webview + PTY + external-CLI integration that cannot be exercised headlessly. Per the approved spec, verification is the TypeScript typecheck plus the manual checklist in Task 3. Do NOT add a test framework for this feature.

**Branch:** work happens on `feat/clipboard-image-paste` (already created off `main`; spec is committed there).

---

### Task 1: Forward image-only paste in Terminal.tsx

**Files:**
- Modify: `src/components/Terminal.tsx` (listener registration ~line 183, cleanup ~line 192)

- [ ] **Step 1: Register the paste handler**

In `src/components/Terminal.tsx`, find this block inside the mount effect:

```ts
    window.addEventListener("ac:term-input", onTermInput as EventListener);

    return () => {
```

Insert the paste handler between the `ac:term-input` registration and the `return () => {` line:

```ts
    window.addEventListener("ac:term-input", onTermInput as EventListener);

    // Image-only paste. The webview fires `paste` with no text, so xterm
    // writes nothing and the agent never sees the keystroke. Forward the
    // Ctrl+V byte (0x16) to the PTY instead: Claude/Codex read the OS
    // clipboard natively when they receive it, exactly as in a native
    // terminal. When the clipboard also carries text we fall through and
    // xterm pastes the text as usual (text wins, like a native terminal).
    // Capture phase so this runs before xterm's own handler on its textarea.
    const onPaste = (e: ClipboardEvent) => {
      const dt = e.clipboardData;
      if (!dt || !termId) return;
      if (dt.getData("text/plain")) return;
      if (!Array.from(dt.items).some((it) => it.type.startsWith("image/"))) return;
      e.preventDefault();
      e.stopPropagation();
      ipc.termWrite(termId, "\x16").catch(() => {});
    };
    host.addEventListener("paste", onPaste, true);

    return () => {
```

Notes for the implementer:
- `termId` is the `let termId: string | null = null` declared at the top of the effect (~line 70); the existing `onTermInput` handler reads it the same way, so closing over it is the established pattern.
- `host` is the captured `hostRef.current` from the top of the effect.
- Do not use `navigator.clipboard.read()` — it needs webview permissions; `e.clipboardData` is synchronous and permission-free.

- [ ] **Step 2: Remove the listener in the teardown**

In the same effect's cleanup, find:

```ts
      window.removeEventListener("ac:clear-terminal", onClear);
      window.removeEventListener("ac:term-input", onTermInput as EventListener);
```

and add the paste cleanup directly after those two lines:

```ts
      window.removeEventListener("ac:clear-terminal", onClear);
      window.removeEventListener("ac:term-input", onTermInput as EventListener);
      host.removeEventListener("paste", onPaste, true);
```

- [ ] **Step 3: Typecheck**

Run from the repo root (`C:\Users\Usuario\work\personal\agent-console`):

```powershell
npx tsc --noEmit
```

Expected: exit code 0, no output.

- [ ] **Step 4: Commit**

```powershell
git add src/components/Terminal.tsx
git commit -m "Terminal: forward image-only paste to the agent PTY"
```

(Per user config: no `Co-Authored-By: Claude` trailer.)

---

### Task 2: Document the shortcut in ShortcutsModal

**Files:**
- Modify: `src/components/ShortcutsModal.tsx:28-36` (the `Workflows` group in `GROUPS`)

- [ ] **Step 1: Add the row**

In `src/components/ShortcutsModal.tsx`, find the `Workflows` group:

```ts
  {
    title: "Workflows",
    rows: [
      ["Ctrl+L", "Clear terminal"],
```

and add the new row directly after `["Ctrl+L", "Clear terminal"],`:

```ts
  {
    title: "Workflows",
    rows: [
      ["Ctrl+L", "Clear terminal"],
      ["Ctrl+V", "Paste clipboard image into the agent"],
```

- [ ] **Step 2: Typecheck**

```powershell
npx tsc --noEmit
```

Expected: exit code 0, no output.

- [ ] **Step 3: Commit**

```powershell
git add src/components/ShortcutsModal.tsx
git commit -m "Shortcuts: document Ctrl+V clipboard image paste"
```

---

### Task 3: Manual verification

**Files:** none (run the app).

- [ ] **Step 1: Launch the dev app**

```powershell
npm run tauri dev
```

Expected: the app window opens; open a project and start a Claude session (terminal spawns, `claude` launches in the PTY).

- [ ] **Step 2: Image-only paste reaches Claude**

Take a screenshot with `Win+Shift+S` (puts an image-only payload on the clipboard), focus the terminal, press `Ctrl+V`.

Expected: Claude Code's composer shows `[Image #1]` (its native pasted-image chip).

- [ ] **Step 3: Image-only paste reaches Codex**

Start a Codex session, repeat Step 2.

Expected: Codex's composer shows its image attachment.

- [ ] **Step 4: Text paste is unchanged**

Copy a single-line string, paste into the terminal; then copy a multiline string, paste again.

Expected: both paste as text exactly as before this change (multiline goes through bracketed paste, no stray `0x16` behavior).

- [ ] **Step 5: Text+image clipboard pastes the text**

Copy rich content that carries both text and image (e.g. select text+image in a browser page and copy).

Expected: the text is pasted; no image is attached.

- [ ] **Step 6: Empty clipboard does nothing**

Clear the clipboard (copy nothing / press `Win+V` and clear), press `Ctrl+V` in the terminal.

Expected: nothing happens, no errors in the dev console.

- [ ] **Step 7: Check off the spec's verification list**

All five checks in the spec's "Verification (manual)" section are covered by Steps 2–6. If any failed, stop and fix before proceeding.

---

## Out of scope (from the spec)

- Drag & drop of image files onto the terminal.
- Fallback for agent CLIs without native clipboard paste (`supportsClipboardImagePaste` profile flag) — revisit only when such an agent is added.
