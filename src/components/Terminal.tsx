import { useEffect, useRef } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import "@xterm/xterm/css/xterm.css";

import { ipc, type TermExit, type TermOutput } from "../ipc/tauri";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useThemeStore } from "../stores/themeStore";
import { profileFor } from "../agents/profiles";

/// Detail for the `ac:term-input` window event: write `data` into the PTY of
/// the session whose id matches. Used by the StatusBar model pill to send
/// `/model <alias>` into a live Claude session without leaking PTY ids.
export interface TermInputDetail {
  sessionId: string;
  data: string;
}

const TERM_THEMES = {
  dark:  { background: "#0d0f12", foreground: "#d9dde3", cursor: "#6aa9ff" },
  light: { background: "#fbfcfd", foreground: "#1a1d23", cursor: "#2563eb" },
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
    const fit = new FitAddon();
    fitRef.current = fit;
    term.loadAddon(fit);
    term.open(host);
    fit.fit();

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
      unlistenOutput = await listen<TermOutput>("term://output", (e) => {
        if (e.payload.id === termId) {
          term.write(e.payload.data);
          appendOutput(session.id, e.payload.data);
        }
      });
      unlistenExit = await listen<TermExit>("term://exit", (e) => {
        if (e.payload.id === termId) {
          term.write(`\r\n\x1b[90m[process exited: ${e.payload.code ?? "?"}]\x1b[0m\r\n`);
        }
      });

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

      try {
        termId = await ipc.termSpawn(session.cwd, session.id);
        if (disposed) {
          await ipc.termKill(termId);
          return;
        }
        markLive(session.id);
        await ipc.termResize(termId, term.cols, term.rows);
      } catch (err) {
        term.write(`\x1b[31mfailed to spawn terminal: ${err}\x1b[0m\r\n`);
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
        setTimeout(() => {
          if (disposed) return;
          term.write(`\x1b[90m── ${launchNote} ${launchLabel} ──\x1b[0m\r\n`);
          ipc.termWrite(tid, `${launchCmd}\r`).catch(() => {});
        }, 600);
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

    return () => {
      disposed = true;
      if (debounceTimer !== null) window.clearTimeout(debounceTimer);
      ro.disconnect();
      unlistenOutput?.();
      unlistenExit?.();
      unlistenDragDrop?.();
      if (termId) ipc.termKill(termId).catch(() => {});
      window.removeEventListener("ac:clear-terminal", onClear);
      window.removeEventListener("ac:term-input", onTermInput as EventListener);
      host.removeEventListener("paste", onPaste, true);
      window.removeEventListener("focus", onFocus);
      term.dispose();
    };
    // session.id is stable; we deliberately don't re-run on cwd/name changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.id]);

  // Re-fit when becoming visible (xterm needs a real size at layout time).
  useEffect(() => {
    if (visible) {
      try { fitRef.current?.fit(); } catch { /* ignore */ }
    }
  }, [visible]);

  // Live-swap xterm theme on toggle so it matches the rest of the UI.
  useEffect(() => {
    const t = termRef.current;
    if (!t) return;
    t.options.theme = TERM_THEMES[theme];
  }, [theme]);

  return (
    <div
      ref={hostRef}
      className="terminal-host"
      style={{ display: visible ? "block" : "none" }}
    />
  );
}
