# Clipboard image paste in the terminal — design

**Date:** 2026-06-09 (revised 2026-06-10 after live debugging)
**Status:** Approved (v2 — temp-file approach)

## Goal

When the user has an image in the system clipboard and presses Ctrl+V (or
Cmd+V on macOS, or uses the context-menu Paste) inside a terminal session,
the running agent CLI (Claude Code, Codex) should receive the image.

## Background

The terminal is xterm.js inside a Tauri 2 webview; keystrokes flow through
`ipc.termWrite` into a portable-pty PTY (`src/components/Terminal.tsx` →
`src-tauri/src/commands/terminal.rs`). An image-only paste did nothing:
the webview's `paste` event carries no text, so xterm wrote nothing.

## Why v1 (forward Ctrl+V to the PTY) was abandoned

v1 forwarded `0x16` (Ctrl+V) to the PTY so the agent CLI would read the OS
clipboard itself. Live CDP instrumentation of the running app proved every
link of that chain works — except the last one:

- The webview `paste` event fires with `items: [file:image/png]` ✓
- The v1 handler ran and wrote `0x16` to the PTY ✓ (verified via synthetic
  paste: `defaultPrevented === true`)
- PTY → claude input delivery works ✓ (text and alt+p both land)
- **claude on Windows ignores `0x16`**: its image-paste binding is
  `alt+v` on win32/WSL (`ctrl+v` elsewhere), per the binding registry in
  the claude binary.
- **`alt+v` could not be delivered at all** through ConPTY: ESC-prefix
  (`\x1bv`) and win32-input-mode sequences (with uChar=0 and =118) all
  failed, while the same encodings deliver `alt+p` (binding `meta+p`)
  successfully. claude's key parser appears to tokenize Alt as `meta`, so
  its own literal `"alt+v"` default binding never matches via ConPTY —
  an upstream claude-code bug on Windows, outside this app's control.

## Approach (v2): save to temp file, type the path

On image-only paste, the webview already holds the image bytes (the
`paste` event's `File`). Save them to a temp file via a small Rust command
and type the quoted path into the composer — exactly what dragging an
image file onto the terminal does, which every supported agent CLI
understands (Claude reads image paths from the prompt; Codex has
`view_image`). Agent-neutral, cross-platform, independent of any CLI
keybinding.

Rejected alternatives:
- v1 key forwarding: see above.
- Tauri clipboard plugin / `navigator.clipboard.read()`: the async
  clipboard API hangs on an invisible permission prompt in WebView2
  (verified live; permission state stays `prompt`), and a clipboard crate
  is unnecessary when the paste event already delivers the bytes.

## Design

### Rust: `term_save_paste_image` (src-tauri/src/commands/terminal.rs)

- `#[tauri::command]` taking a raw-body `tauri::ipc::Request` (the image
  bytes as `InvokeBody::Raw` — no JSON array overhead, no base64 dep).
- Optional `x-image-ext` request header selects the extension from an
  allowlist (`png`, `jpg`, `gif`, `webp`, `bmp`); anything else → `png`.
- Writes `agent-console-paste-<millis>-<counter>.<ext>` to
  `std::env::temp_dir()`, returns the absolute path.
- Empty/non-raw body → `AppError::InvalidArgument`.
- Temp files are left to the OS temp dir (same policy as claude's own
  pasted-image temp files); no cleanup pass. Registered in `lib.rs`.

### Frontend (src/components/Terminal.tsx + src/ipc/tauri.ts)

`ipc.termSavePasteImage(bytes, ext)` wraps the command, passing bytes as
`Uint8Array` and the ext via header.

The capture-phase `paste` listener on the xterm host (kept from v1):

- Clipboard has text → do nothing; xterm pastes text as today (text wins,
  like a native terminal).
- No text, first `image/*` item present → `preventDefault()` +
  `stopPropagation()`, then async: `File.arrayBuffer()` →
  `termSavePasteImage` → `termWrite(termId, '"<path>" ')`.
- Any failure → silent (`.catch(() => {})`), consistent with the file.
- Listener removed in the effect teardown.

### Discoverability

`ShortcutsModal.tsx` row (kept from v1):
`Ctrl+V — Paste clipboard image into the agent`.

## Verification (manual)

1. Screenshot (Win+Shift+S) → Ctrl+V in a Claude session → the quoted
   temp-file path appears in the composer; submitting a prompt about it
   makes claude read the image.
2. Same in a Codex session.
3. Plain and multiline text paste still work unchanged.
4. Clipboard holding both text and image pastes the text.
5. Ctrl+V with an empty clipboard does nothing.

## Out of scope

- Drag & drop of image files onto the terminal.
- Cleanup of generated temp files.
- Upstream claude-code `alt+v`-on-Windows bug (report separately).
