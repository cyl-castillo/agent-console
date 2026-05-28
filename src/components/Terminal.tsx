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

      // Auto-launch claude. Only do precise --resume when this terminal has a
      // captured claudeSessionId (from the UserPromptSubmit hook). Never use
      // `claude --continue` as a fallback: it picks the LAST claude session
      // globally, so multiple stopped terminals would all converge on the
      // same conversation. When the id is missing we just start `claude`
      // fresh — the user can run `/resume` inside claude to pick one.
      if (termId) {
        let cmd: string;
        let label: string;
        let note: string;
        if (session.claudeSessionId) {
          cmd = `claude --resume ${session.claudeSessionId}`;
          label = `claude (${session.claudeSessionId.slice(0, 8)}…)`;
          note = "auto-resuming";
        } else {
          cmd = "claude";
          label = "claude";
          note = session.initialScrollback ? "starting fresh (no session id)" : "starting";
        }
        const tid = termId;
        const launchCmd = cmd;
        const launchLabel = label;
        const launchNote = note;
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

    // Refit only when the host's outer size actually changed. Without the
    // dimension guard, conpty (Windows) loops: each resize triggers Claude to
    // redraw its UI, the redraw nudges xterm's inner DOM by sub-pixel amounts,
    // ResizeObserver fires again, fit() recomputes a slightly different
    // cols/rows, conpty resizes again, repeat — visible as the Claude banner
    // jumping/redrawing constantly.
    let rafPending = false;
    let lastW = 0;
    let lastH = 0;
    const ro = new ResizeObserver(() => {
      if (rafPending) return;
      const rect = host.getBoundingClientRect();
      if (rect.width < 8 || rect.height < 8) return;
      // Ignore tiny fluctuations (rounding / sub-pixel layout shifts).
      if (Math.abs(rect.width - lastW) < 2 && Math.abs(rect.height - lastH) < 2) return;
      lastW = rect.width; lastH = rect.height;
      rafPending = true;
      requestAnimationFrame(() => {
        rafPending = false;
        try { fit.fit(); } catch { /* host not mounted yet */ }
      });
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
