import { useEffect, useRef, useState } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import "@xterm/xterm/css/xterm.css";

import { ipc, type TermExit, type TermOutput } from "../ipc/tauri";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useThemeStore } from "../stores/themeStore";
import { useToastStore } from "../stores/toastStore";
import { profileFor } from "../agents/profiles";
import { clipboardActionFor } from "./terminalClipboard";
import {
  readText as clipboardReadText,
  writeText as clipboardWriteText,
} from "@tauri-apps/plugin-clipboard-manager";

/// Detail for the `ac:term-input` window event: write `data` into the PTY of
/// the session whose id matches. Used by the StatusBar model pill to send
/// `/model <alias>` into a live Claude session without leaking PTY ids.
export interface TermInputDetail {
  sessionId: string;
  data: string;
}

/// Clipboard IO: the native Tauri plugin first (WebKitGTK rejects
/// navigator.clipboard writes on Linux), the web API as fallback elsewhere.
function writeClip(text: string): Promise<void> {
  return clipboardWriteText(text).catch(() => navigator.clipboard.writeText(text));
}
function readClip(): Promise<string> {
  return clipboardReadText().catch(() => navigator.clipboard.readText());
}

const TERM_THEMES = {
  // selectionBackground is explicit so the highlight is clearly visible in both
  // themes — an invisible selection reads as "copy is broken".
  dark:  { background: "#0d0f12", foreground: "#d9dde3", cursor: "#6aa9ff", selectionBackground: "rgba(106, 169, 255, 0.35)" },
  light: { background: "#fbfcfd", foreground: "#1a1d23", cursor: "#2563eb", selectionBackground: "rgba(37, 99, 235, 0.25)" },
} as const;

interface Props {
  session: TerminalSession;
  visible: boolean;
}

/// Single PTY-backed terminal owned by one TerminalSession. Stays mounted
/// while the session exists (visibility toggled via CSS so the PTY survives
/// tab/session switches). Tear-down only happens when the session is closed.
export function Terminal({ session, visible }: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const appendOutput = useTerminalsStore((s) => s.appendOutput);
  const markLive = useTerminalsStore((s) => s.markLive);
  const fitRef = useRef<FitAddon | null>(null);
  const termRef = useRef<XTerm | null>(null);
  const theme = useThemeStore((s) => s.theme);
  // Right-click context menu: users reach for right-click → Copy before any
  // keyboard shortcut, and the webview's own menu can't see xterm's internal
  // selection (it always shows Copy disabled). hasSelection is snapshotted at
  // open time so the Copy item enables/disables correctly.
  const [menu, setMenu] = useState<{ x: number; y: number; hasSelection: boolean } | null>(null);
  // Last non-empty selection, captured the moment it's made (onSelectionChange)
  // — not at click time. xterm clears its selection on the next pointer event,
  // so any click-time read races against that; this never loses.
  const selSnapshotRef = useRef<{ text: string; ts: number }>({ text: "", ts: 0 });

  // Spawn once per session id.
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new XTerm({
      fontFamily: '"JetBrains Mono", ui-monospace, monospace',
      fontSize: 13,
      theme: TERM_THEMES[useThemeStore.getState().theme],
      cursorBlink: true,
      convertEol: true,
      scrollback: 5000,
    });
    termRef.current = term;

    // Copy/paste: xterm's selection is internal (no native DOM selection), so
    // without this the app has NO copy path at all — Ctrl+C goes to the PTY as
    // SIGINT and the webview context menu never offers Copy. Policy lives in
    // clipboardActionFor (terminalClipboard.ts): Ctrl/Cmd+Shift+C copies,
    // plain Ctrl/Cmd+C copies only when a selection is active (then clears it,
    // so the next Ctrl+C is SIGINT again), Ctrl/Cmd+Shift+V pastes.
    // The clipboard itself goes through the native Tauri plugin — WebKitGTK
    // (Linux webview) rejects navigator.clipboard writes, which made the first
    // version of this fix silently do nothing. navigator.clipboard stays as
    // the fallback for platforms where the plugin call fails.
    term.attachCustomKeyEventHandler((e) => {
      const action = clipboardActionFor(e, term.hasSelection());
      if (!action) return true;
      if (action === "paste") {
        readClip()
          .then((text) => { if (text) term.paste(text); })
          .catch(() => {});
        return false;
      }
      // Ctrl+Shift+C may fall back to the last-selection snapshot; plain
      // Ctrl+C only ever gets here with a live selection (policy above).
      const sel = term.getSelection() ||
        (action === "copy" ? selSnapshotRef.current.text : "");
      if (sel) {
        writeClip(sel).catch(() => {
          useToastStore.getState().show("Copy failed — clipboard unavailable", "error");
        });
        if (action === "copy-and-clear") term.clearSelection();
      }
      return false;
    });

    const fit = new FitAddon();
    fitRef.current = fit;
    term.loadAddon(fit);
    term.open(host);
    fit.fit();
    // The synchronous fit() above runs before the layout has settled AND before
    // the web font has loaded, so FitAddon measures the wrong height and/or the
    // wrong cell height — it then reports one row too many to the PTY. The agent
    // draws that extra row below the host's content-box, where overflow:hidden
    // clips it flush against the status bar (it reads as the bar "covering" the
    // last line). Nothing ever corrects it because no resize follows. So refit
    // (a) on the next frame, once the grid row has its real height, and (b) once
    // the terminal font is ready, so the cell metrics — and thus the row count —
    // match what's actually visible.
    const rafFit = requestAnimationFrame(() => {
      try { fit.fit(); } catch { /* host not mounted yet */ }
    });
    let fontsDone = false;
    document.fonts.ready.then(() => {
      if (disposed || fontsDone) return;
      fontsDone = true;
      try { fit.fit(); } catch { /* host not mounted yet */ }
    });

    // Replay saved scrollback from a previous run, dimmed, with a divider.
    if (session.initialScrollback) {
      term.write(
        `\x1b[90m── previous session (${new Date(session.createdAtMs).toLocaleString()}) ──\x1b[0m\r\n`,
      );
      term.write(session.initialScrollback);
      // The saved scrollback is the raw tail of a live agent TUI, so it carries
      // the mode-ENABLE sequences (mouse tracking, focus reporting, bracketed
      // paste, alt-screen) but not the matching disables — those only fire on a
      // clean exit we never captured. Replaying it leaves the fresh xterm with
      // those modes stuck on; with any-event mouse tracking (\x1b[?1003h) that
      // means every mouse move floods the bare shell with SGR reports it tries
      // to run as commands ("command not found"). Reset the input-affecting
      // modes here so the terminal is sane while the shell holds the prompt;
      // the relaunched agent re-enables whatever it needs on startup.
      term.write(
        "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1006l\x1b[?2004l\x1b[?1049l",
      );
      term.write(`\r\n\x1b[90m── resumed ──\x1b[0m\r\n`);
    }

    let termId: string | null = null;
    let unlistenOutput: UnlistenFn | null = null;
    let unlistenExit: UnlistenFn | null = null;
    let unlistenDragDrop: UnlistenFn | null = null;
    let disposed = false;

    (async () => {
      // Each `await listen` can resolve AFTER the component unmounted (fast
      // close, StrictMode). Assign-then-forget leaked one listener per lost
      // race; check `disposed` at every resolution point instead.
      const uOut = await listen<TermOutput>("term://output", (e) => {
        if (e.payload.id === termId) {
          term.write(e.payload.data);
          appendOutput(session.id, e.payload.data);
        }
      });
      if (disposed) { uOut(); return; }
      unlistenOutput = uOut;
      const uExit = await listen<TermExit>("term://exit", (e) => {
        if (e.payload.id === termId) {
          term.write(`\r\n\x1b[90m[process exited: ${e.payload.code ?? "?"}]\x1b[0m\r\n`);
        }
      });
      if (disposed) { uExit(); return; }
      unlistenExit = uExit;

      // Drop files/folders onto this terminal: type their quoted paths into
      // the composer (same flow as the clipboard-image paste below). Tauri
      // intercepts webview drag&drop (dragDropEnabled defaults to true), so
      // HTML5 drop events never fire and this webview event — which carries
      // real OS paths — is the only source. Every mounted Terminal listens;
      // the visibility + hit-test gate picks the one under the cursor.
      const uDrop = await getCurrentWebview().onDragDropEvent((e) => {
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
      if (disposed) { uDrop(); return; }
      unlistenDragDrop = uDrop;

      try {
        termId = await ipc.termSpawn(session.cwd, session.id);
        if (disposed) {
          await ipc.termKill(termId);
          return;
        }
        markLive(session.id);
        await ipc.termResize(termId, term.cols, term.rows);
      } catch (err) {
        term.write(`\x1b[31m✕ Couldn't start a shell session.\x1b[0m\r\n`);
        term.write(`\x1b[90m  ${err}\x1b[0m\r\n`);
        term.write(`\x1b[90m  Check that your shell is available, then open a new session to retry.\x1b[0m\r\n`);
        useToastStore.getState().show(`Couldn't start a terminal session: ${err}`, "error");
        return;
      }

      // Auto-launch the session's agent. The profile owns how the command is
      // built (resume strategy, model/tuning encoding) and validates that any
      // interpolated model value is shell-safe before it reaches the PTY. We
      // type the command as text into the login-shell PTY, which resolves the
      // bare binary (`claude`/`codex`) from the user's PATH.
      if (termId) {
        const { cmd: launchCmd, label: launchLabel, note: launchNote } =
          profileFor(session.agent).buildLaunch({
            agentSessionId: session.claudeSessionId,
            model: session.model,
            hasScrollback: Boolean(session.initialScrollback),
          });
        const tid = termId;
        // Fresh worktree sessions may carry a one-shot install command (from
        // .claude/worktree-setup.json). Chain it before the agent launch so it
        // runs visibly in this terminal and the agent only starts if it succeeds.
        const setup = session.setupCmd?.trim();
        const cmd = setup ? `${setup} && ${launchCmd}` : launchCmd;
        setTimeout(() => {
          if (disposed) return;
          if (setup) term.write(`\x1b[90m── workspace setup: ${setup} ──\x1b[0m\r\n`);
          term.write(`\x1b[90m── ${launchNote} ${launchLabel} ──\x1b[0m\r\n`);
          ipc.termWrite(tid, `${cmd}\r`).catch(() => {});
        }, 600);

        // First-spawn prompt seed (e.g. a Jira ticket): typed into the agent's
        // input once its TUI has had time to boot, and WITHOUT a trailing
        // newline so the human reviews and submits it. A setup command runs
        // first and takes longer, so wait more when one is present.
        const seed = session.seedPrompt?.trim();
        if (seed) {
          setTimeout(() => {
            if (disposed) return;
            ipc.termWrite(tid, seed).catch(() => {});
          }, setup ? 6000 : 3000);
        }
      }

      term.onData((data) => {
        if (termId) ipc.termWrite(termId, data).catch(() => {});
      });
      // Dedupe resize calls: conpty (Windows) re-emits the whole buffer on
      // resize, and that redraw can change DOM metrics, which would re-trigger
      // ResizeObserver in a loop. Only send when cols/rows actually changed.
      let lastCols = -1;
      let lastRows = -1;
      term.onResize(({ cols, rows }) => {
        if (!termId) return;
        if (cols === lastCols && rows === lastRows) return;
        lastCols = cols; lastRows = rows;
        ipc.termResize(termId, cols, rows).catch(() => {});
      });
    })();

    // Conservative resize handling. ResizeObserver fires for any sub-pixel
    // layout change (window blur/focus, compositor redraws, agent UI redraws).
    // To keep the agent's TUI from redrawing visibly while the user drags the
    // window:
    //   - require 800ms of quiet before calling fit(), so a drag only reflows
    //     the terminal once the user stops,
    //   - skip while the window has no focus, and refit on focus return.
    // The delta gate only suppresses true sub-pixel jitter, so it must stay
    // BELOW one character cell (~8px wide, ~16px tall). A larger gate (it was
    // 16px) lets xterm's grid drift 1–2 columns off the real pane without ever
    // refitting; the agent then draws its full-width footer onto a grid that's
    // a couple columns too narrow and the text overlaps itself. 4px is under a
    // cell, so any change that can shift a row/column still triggers a refit.
    let debounceTimer: number | null = null;
    let lastW = 0;
    let lastH = 0;
    const scheduleFit = () => {
      if (debounceTimer !== null) window.clearTimeout(debounceTimer);
      debounceTimer = window.setTimeout(() => {
        debounceTimer = null;
        if (disposed) return;
        try { fit.fit(); } catch { /* host not mounted yet */ }
      }, 800);
    };
    const ro = new ResizeObserver(() => {
      const rect = host.getBoundingClientRect();
      if (rect.width < 8 || rect.height < 8) return;
      if (!document.hasFocus()) return;
      if (Math.abs(rect.width - lastW) < 4 && Math.abs(rect.height - lastH) < 4) return;
      lastW = rect.width; lastH = rect.height;
      scheduleFit();
    });
    ro.observe(host);
    const onFocus = () => scheduleFit();
    window.addEventListener("focus", onFocus);

    const onClear = () => { term.clear(); };
    window.addEventListener("ac:clear-terminal", onClear);

    // External input (e.g. the StatusBar model pill sending `/model <alias>`).
    // Only act if the event targets this session and our PTY is live.
    const onTermInput = (e: Event) => {
      const detail = (e as CustomEvent<TermInputDetail>).detail;
      if (!detail || detail.sessionId !== session.id || !termId) return;
      ipc.termWrite(termId, detail.data).catch(() => {});
    };
    window.addEventListener("ac:term-input", onTermInput as EventListener);

    // Refocus the terminal keyboard (e.g. after the last pending approval is
    // decided) so the flow continues where the interruption started. Only the
    // active session answers.
    const onFocusTerminal = () => {
      if (useTerminalsStore.getState().activeId === session.id) term.focus();
    };
    window.addEventListener("ac:focus-terminal", onFocusTerminal);

    // Image-only paste. The webview fires `paste` with no text, so xterm
    // writes nothing. Forwarding the keystroke to the agent is a dead end:
    // Claude on Windows binds image paste to alt+v, which ConPTY input can't
    // deliver (see docs/superpowers/specs/2026-06-09-clipboard-image-paste-
    // design.md). Instead, save the pasted bytes to a temp file and type its
    // quoted path into the composer — the drag-and-drop flow every agent CLI
    // already understands. When the clipboard also carries text we fall
    // through and xterm pastes the text as usual (text wins, like a native
    // terminal). Capture phase so this runs before xterm's own handler.
    const onPaste = (e: ClipboardEvent) => {
      const dt = e.clipboardData;
      if (!dt || !termId) return;
      if (dt.getData("text/plain")) return;
      const file = Array.from(dt.items)
        .find((it) => it.type.startsWith("image/"))
        ?.getAsFile();
      if (!file) return;
      e.preventDefault();
      e.stopPropagation();
      const subtype = file.type.split("/")[1] ?? "png";
      const ext = subtype === "jpeg" ? "jpg" : subtype;
      const tid = termId;
      file
        .arrayBuffer()
        .then((buf) => ipc.termSavePasteImage(new Uint8Array(buf), ext))
        .then((path) => ipc.termWrite(tid, `"${path}" `))
        .catch(() => {});
    };
    host.addEventListener("paste", onPaste, true);

    // Capture every non-empty selection the moment it's made. Reading the
    // selection at click time is a lost race — xterm clears it on the next
    // pointerdown (which fires before mousedown), and the agent TUI's
    // repaints can clear it even earlier. This way Copy always has the last
    // thing the user selected.
    const selDisposable = term.onSelectionChange(() => {
      const text = term.getSelection();
      if (text) selSnapshotRef.current = { text, ts: Date.now() };
    });

    // Right-clicks must never reach xterm (pointerdown AND mousedown — v6
    // listens on pointer events, which fire first): its press handling
    // clears the active selection right as the user reaches for Copy.
    const onRightDown = (e: PointerEvent | MouseEvent) => {
      if (e.button === 2) e.stopPropagation();
    };
    host.addEventListener("pointerdown", onRightDown, true);
    host.addEventListener("mousedown", onRightDown, true);

    return () => {
      disposed = true;
      cancelAnimationFrame(rafFit);
      if (debounceTimer !== null) window.clearTimeout(debounceTimer);
      ro.disconnect();
      unlistenOutput?.();
      unlistenExit?.();
      unlistenDragDrop?.();
      if (termId) ipc.termKill(termId).catch(() => {});
      window.removeEventListener("ac:clear-terminal", onClear);
      window.removeEventListener("ac:term-input", onTermInput as EventListener);
      window.removeEventListener("ac:focus-terminal", onFocusTerminal);
      host.removeEventListener("paste", onPaste, true);
      host.removeEventListener("pointerdown", onRightDown, true);
      host.removeEventListener("mousedown", onRightDown, true);
      selDisposable.dispose();
      window.removeEventListener("focus", onFocus);
      term.dispose();
    };
    // session.id is stable; we deliberately don't re-run on cwd/name changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.id]);

  // Re-fit when becoming visible (xterm needs a real size at layout time).
  // Defer to the next frame so the tab-pane has its laid-out height before we
  // measure — fitting against a stale height over-counts rows and clips the
  // last line behind the status bar.
  useEffect(() => {
    if (!visible) return;
    const raf = requestAnimationFrame(() => {
      try { fitRef.current?.fit(); } catch { /* ignore */ }
    });
    return () => cancelAnimationFrame(raf);
  }, [visible]);

  // Live-swap xterm theme on toggle so it matches the rest of the UI.
  useEffect(() => {
    const t = termRef.current;
    if (!t) return;
    t.options.theme = TERM_THEMES[theme];
  }, [theme]);

  // Close the context menu on any outside press or Escape.
  useEffect(() => {
    if (!menu) return;
    const onDown = (e: MouseEvent) => {
      if (!(e.target as HTMLElement | null)?.closest?.(".term-menu")) setMenu(null);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setMenu(null); };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  const menuCopy = () => {
    const t = termRef.current;
    // Prefer the live selection; fall back to the snapshot taken at
    // right-mousedown (xterm may have cleared the visual selection since).
    const sel = t?.getSelection() || selSnapshotRef.current.text;
    if (sel) {
      writeClip(sel)
        .then(() => useToastStore.getState().show("Copied", "success"))
        .catch(() => {
          useToastStore.getState().show("Copy failed — clipboard unavailable", "error");
        });
    }
    setMenu(null);
    t?.focus();
  };
  const menuPaste = () => {
    const t = termRef.current;
    readClip()
      .then((text) => { if (text && t) t.paste(text); })
      .catch(() => {});
    setMenu(null);
    t?.focus();
  };
  const menuSelectAll = () => {
    termRef.current?.selectAll();
    setMenu(null);
  };

  return (
    <>
      <div
        ref={hostRef}
        className="terminal-host"
        style={{ display: visible ? "block" : "none" }}
        onContextMenu={(e) => {
          e.preventDefault();
          const live = termRef.current?.hasSelection() ?? false;
          // A recent snapshot counts: the selection was made moments ago even
          // if something (repaint, pointerdown) already cleared the visual.
          const snap = selSnapshotRef.current.text.length > 0 &&
            Date.now() - selSnapshotRef.current.ts < 15_000;
          setMenu({
            // Clamp so the menu never opens off-screen near the edges.
            x: Math.min(e.clientX, window.innerWidth - 190),
            y: Math.min(e.clientY, window.innerHeight - 130),
            hasSelection: live || snap,
          });
        }}
      />
      {menu && (
        <div className="term-menu" style={{ left: menu.x, top: menu.y }} role="menu">
          <button role="menuitem" disabled={!menu.hasSelection} onClick={menuCopy}>
            <span>Copy</span><kbd>Ctrl+Shift+C</kbd>
          </button>
          <button role="menuitem" onClick={menuPaste}>
            <span>Paste</span><kbd>Ctrl+Shift+V</kbd>
          </button>
          <div className="term-menu-sep" />
          <button role="menuitem" onClick={menuSelectAll}>
            <span>Select all</span>
          </button>
          {!menu.hasSelection && (
            // Agent TUIs turn on mouse tracking (plain drag goes to the app,
            // not to selection) — teach the standard escape hatch right where
            // the user hits the wall.
            <div className="term-menu-hint">Tip: Shift+drag selects text</div>
          )}
        </div>
      )}
    </>
  );
}
