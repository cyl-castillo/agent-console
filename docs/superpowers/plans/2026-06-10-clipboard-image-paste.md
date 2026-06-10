# Clipboard Image Paste Implementation Plan (v2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the clipboard holds an image and no text, Ctrl+V in a terminal session saves the image to a temp file and types its quoted path into the agent composer (same flow as drag & drop).

**Architecture:** The capture-phase `paste` listener in `Terminal.tsx` (from v1) keeps its text-wins/image-only gating, but instead of forwarding `0x16` it reads the pasted `File` bytes and calls a new raw-body Tauri command `term_save_paste_image`, then `termWrite`s the returned path quoted. v1's key-forwarding was abandoned: claude on Windows binds image paste to `alt+v`, which is undeliverable through ConPTY (see spec v2 for the evidence).

**Tech Stack:** React 19 + TypeScript, xterm.js 6, Tauri 2 IPC raw-body request. No new dependencies (std-only Rust).

**Spec:** `docs/superpowers/specs/2026-06-09-clipboard-image-paste-design.md` (v2)

**Testing note:** No test framework in the repo; verification is `tsc` + the cargo build from the running `tauri dev` + live CDP synthetic-paste check + the manual checklist (Task 3).

**Branch:** `feat/clipboard-image-paste`.

---

### Task 1: Rust command `term_save_paste_image`

**Files:**
- Modify: `src-tauri/src/commands/terminal.rs` (append command)
- Modify: `src-tauri/src/lib.rs` (register after `term_kill`)

- [ ] **Step 1: Append to terminal.rs**

```rust
/// Save image bytes pasted into a terminal to a temp file and return its
/// absolute path. The frontend then types that path into the agent composer
/// (same flow as dragging an image file onto the terminal). Raw-body command:
/// the bytes arrive as `InvokeBody::Raw`, the extension via the
/// `x-image-ext` header (allowlisted, defaults to png).
#[tauri::command]
pub fn term_save_paste_image(request: tauri::ipc::Request<'_>) -> AppResult<String> {
    use std::sync::atomic::{AtomicU64, Ordering};

    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(crate::error::AppError::InvalidArgument(
            "expected raw image bytes".into(),
        ));
    };
    if bytes.is_empty() {
        return Err(crate::error::AppError::InvalidArgument(
            "empty image payload".into(),
        ));
    }
    let ext = request
        .headers()
        .get("x-image-ext")
        .and_then(|v| v.to_str().ok())
        .filter(|e| matches!(*e, "png" | "jpg" | "gif" | "webp" | "bmp"))
        .unwrap_or("png");

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("agent-console-paste-{millis}-{n}.{ext}"));
    std::fs::write(&path, bytes)?;
    Ok(path.to_string_lossy().to_string())
}
```

- [ ] **Step 2: Register in lib.rs** — add `commands::terminal::term_save_paste_image,` after `commands::terminal::term_kill,`.

- [ ] **Step 3: Verify build** — the running `tauri dev` recompiles on save; check its log (or `cargo check` in `src-tauri/`) for errors.

- [ ] **Step 4: Commit**

```powershell
git add src-tauri/src/commands/terminal.rs src-tauri/src/lib.rs
git commit -m "Terminal: add term_save_paste_image raw-body command"
```

---

### Task 2: Frontend — save bytes + type path

**Files:**
- Modify: `src/ipc/tauri.ts` (add wrapper next to termWrite)
- Modify: `src/components/Terminal.tsx` (rework onPaste body)

- [ ] **Step 1: tauri.ts wrapper**

```ts
  termSavePasteImage: (bytes: Uint8Array, ext: string) =>
    invoke<string>("term_save_paste_image", bytes, {
      headers: { "x-image-ext": ext },
    }),
```

- [ ] **Step 2: Rework the onPaste handler in Terminal.tsx** — keep
registration/teardown; replace the body so the image-only branch reads the
`File`, saves it, and types the quoted path (see spec v2 Design for the
exact behavior; final code lives in the component).

- [ ] **Step 3: Typecheck** — `node_modules/.bin/tsc --noEmit` → exit 0.

- [ ] **Step 4: Commit**

```powershell
git add src/ipc/tauri.ts src/components/Terminal.tsx
git commit -m "Terminal: paste clipboard images as temp-file paths"
```

---

### Task 3: Verification

- [ ] **Step 1: CDP synthetic check** — dispatch a synthetic `paste` event
carrying a PNG `File` on the xterm textarea of a live session; expect the
quoted `agent-console-paste-*.png` path to appear in the agent composer and
the file to exist on disk.

- [ ] **Step 2: Manual checklist** (user): screenshot → Ctrl+V in Claude
session shows the quoted path (submit a prompt to confirm claude reads it);
same in Codex; text paste unchanged; text+image pastes text; empty
clipboard no-ops.
