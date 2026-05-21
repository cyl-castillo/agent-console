import { useEffect, useRef } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import "@xterm/xterm/css/xterm.css";

import { ipc, type TermExit, type TermOutput } from "../ipc/tauri";

interface Props {
  cwd: string;
}

/// Single PTY-backed terminal. Owns its xterm instance and Tauri listeners.
/// Killing the terminal is handled on unmount.
export function Terminal({ cwd }: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new XTerm({
      fontFamily: '"JetBrains Mono", ui-monospace, monospace',
      fontSize: 13,
      theme: {
        background: "#0d0f12",
        foreground: "#d9dde3",
        cursor: "#6aa9ff",
      },
      cursorBlink: true,
      convertEol: true,
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();

    let termId: string | null = null;
    let unlistenOutput: UnlistenFn | null = null;
    let unlistenExit: UnlistenFn | null = null;
    let disposed = false;

    (async () => {
      // Listeners BEFORE spawn so we don't miss the first bytes.
      unlistenOutput = await listen<TermOutput>("term://output", (e) => {
        if (e.payload.id === termId) term.write(e.payload.data);
      });
      unlistenExit = await listen<TermExit>("term://exit", (e) => {
        if (e.payload.id === termId) {
          term.write(`\r\n\x1b[90m[process exited: ${e.payload.code ?? "?"}]\x1b[0m\r\n`);
        }
      });

      try {
        termId = await ipc.termSpawn(cwd);
        if (disposed) {
          await ipc.termKill(termId);
          return;
        }
        await ipc.termResize(termId, term.cols, term.rows);
      } catch (err) {
        term.write(`\x1b[31mfailed to spawn terminal: ${err}\x1b[0m\r\n`);
        return;
      }

      // Pipe keystrokes to the PTY.
      term.onData((data) => {
        if (termId) ipc.termWrite(termId, data).catch(() => {});
      });
      // Keep PTY size in sync with the xterm grid.
      term.onResize(({ cols, rows }) => {
        if (termId) ipc.termResize(termId, cols, rows).catch(() => {});
      });
    })();

    // Resize on host element changes.
    const ro = new ResizeObserver(() => {
      try { fit.fit(); } catch { /* host not mounted yet */ }
    });
    ro.observe(host);

    // Global Ctrl+L clear listener.
    const onClear = () => { term.clear(); };
    window.addEventListener("ac:clear-terminal", onClear);

    return () => {
      disposed = true;
      ro.disconnect();
      unlistenOutput?.();
      unlistenExit?.();
      if (termId) ipc.termKill(termId).catch(() => {});
      window.removeEventListener("ac:clear-terminal", onClear);
      term.dispose();
    };
  }, [cwd]);

  return <div ref={hostRef} className="terminal-host" />;
}
