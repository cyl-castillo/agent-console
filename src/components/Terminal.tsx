import { useEffect, useRef } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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
      term.write(`\r\n\x1b[90m── resumed ──\x1b[0m\r\n`);
    }

    let termId: string | null = null;
    let unlistenOutput: UnlistenFn | null = null;
    let unlistenExit: UnlistenFn | null = null;
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

    // Refit conservatively. ResizeObserver fires for any sub-pixel layout
    // change (window blur/focus, compositor redraws, Claude UI redraws). To
    // avoid jumping:
    //   1. ignore changes smaller than 4px,
    //   2. debounce to coalesce rapid drags into one fit at the end,
    //   3. skip while the window isn't focused (we'll catch up on focus).
    // Conservative resize handling. ResizeObserver fires for any sub-pixel
    // layout change. To keep Claude's TUI from redrawing visibly while the
    // user drags the window:
    //   - ignore tiny deltas (<16px ≈ 2 char cols),
    //   - require 800ms of quiet before calling fit(), so a drag only
    //     reflows the terminal once the user stops,
    //   - skip while the window has no focus, and refit on focus return.
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
      if (Math.abs(rect.width - lastW) < 16 && Math.abs(rect.height - lastH) < 16) return;
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

    return () => {
      disposed = true;
      if (debounceTimer !== null) window.clearTimeout(debounceTimer);
      ro.disconnect();
      unlistenOutput?.();
      unlistenExit?.();
      if (termId) ipc.termKill(termId).catch(() => {});
      window.removeEventListener("ac:clear-terminal", onClear);
      window.removeEventListener("ac:term-input", onTermInput as EventListener);
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
