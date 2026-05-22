import { useEffect, useRef } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import "@xterm/xterm/css/xterm.css";

import { ipc, type TermExit, type TermOutput } from "../ipc/tauri";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";

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

  // Spawn once per session id.
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
        termId = await ipc.termSpawn(session.cwd);
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

      term.onData((data) => {
        if (termId) ipc.termWrite(termId, data).catch(() => {});
      });
      term.onResize(({ cols, rows }) => {
        if (termId) ipc.termResize(termId, cols, rows).catch(() => {});
      });
    })();

    const ro = new ResizeObserver(() => {
      try { fit.fit(); } catch { /* host not mounted yet */ }
    });
    ro.observe(host);

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
    // session.id is stable; we deliberately don't re-run on cwd/name changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.id]);

  // Re-fit when becoming visible (xterm needs a real size at layout time).
  useEffect(() => {
    if (visible) {
      try { fitRef.current?.fit(); } catch { /* ignore */ }
    }
  }, [visible]);

  return (
    <div
      ref={hostRef}
      className="terminal-host"
      style={{ display: visible ? "block" : "none" }}
    />
  );
}
