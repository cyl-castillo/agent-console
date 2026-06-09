# Clipboard image paste in the terminal — design

**Date:** 2026-06-09
**Status:** Approved

## Goal

When the user has an image in the system clipboard and presses Ctrl+V (or
Cmd+V on macOS, or uses the context-menu Paste) inside a terminal session,
the running agent CLI (Claude Code, Codex) should receive the image, exactly
as it would in a native terminal like Windows Terminal or iTerm2.

## Background

The terminal is xterm.js inside a Tauri 2 webview; keystrokes flow through
`ipc.termWrite` into a portable-pty PTY (`src/components/Terminal.tsx` →
`src-tauri/src/commands/terminal.rs`). Today an image-only paste does
nothing: the webview's `paste` event carries no text, xterm writes nothing,
and the Ctrl+V keypress never reaches the agent.

Both supported agent CLIs already implement clipboard image paste natively:
when they receive the Ctrl+V byte (`0x16`) on stdin, they read the system
clipboard with OS APIs, stash the image, and show their own UX
(`[Image #1]` in Claude Code, the composer attachment in Codex). The app is
swallowing the key before they ever see it.

## Approach (chosen: forward Ctrl+V to the PTY)

Considered alternatives:

- **A. Forward `0x16` to the PTY** when the clipboard holds an image —
  each CLI handles the clipboard itself with its native UX. *Chosen.*
- **B. Host-side temp file:** save the pasted image to a temp PNG via a
  Rust command and type the quoted path into the prompt. Rejected: temp
  file lifecycle, path quoting, and a worse UX than the CLIs' native one.
- **C. Hybrid A+B** behind a per-agent profile flag. Rejected as YAGNI:
  both current agents support clipboard paste natively.

Approach A keeps the app fully agent-neutral (no per-agent code, no change
to `profiles.ts`) and matches real-terminal behaviour.

## Design

All changes live in `src/components/Terminal.tsx`. No Rust changes, no new
IPC.

### Data flow

1. Register a `paste` listener in the **capture phase** on the xterm host
   element, alongside the other listeners in the mount effect.
2. On paste, inspect the synchronous `clipboardData` (no webview
   permissions needed, unlike `navigator.clipboard.read()`):
   - **Clipboard has text** → do nothing; xterm pastes the text as today
     (bracketed paste included). Text wins when both text and image are
     present — same as a native terminal.
   - **No text, but an `image/*` item is present** → `preventDefault()` +
     `stopPropagation()`, then `ipc.termWrite(termId, "\x16")`.
3. The agent CLI receives Ctrl+V on stdin, reads the image from the system
   clipboard natively, and shows its own feedback.

This covers Ctrl+V, Cmd+V (macOS), and context-menu Paste, since all of
them fire the DOM `paste` event.

### Error handling

- `termWrite` failures are ignored (`.catch(() => {})`), consistent with
  the rest of the file.
- If the PTY is running a bare shell (no agent), `0x16` is harmless — same
  as a native terminal.
- Listener removed in the effect teardown.

### Discoverability

Add one row to `ShortcutsModal.tsx`:
`Ctrl+V — Paste clipboard image into the agent`.

## Verification (manual)

The project has no test framework, and this is webview + PTY + CLI
integration, so verification is manual:

1. Screenshot (Win+Shift+S) → Ctrl+V in a Claude session → `[Image #1]`
   appears in the composer.
2. Same in a Codex session.
3. Plain and multiline text paste still work unchanged.
4. Clipboard holding both text and image pastes the text.
5. Ctrl+V with an empty clipboard does nothing.

## Out of scope

- Drag & drop of image files onto the terminal.
- Fallback for agent CLIs without native clipboard support (revisit with a
  profile flag if such an agent is added).
