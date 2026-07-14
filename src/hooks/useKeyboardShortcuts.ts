import { useEffect } from "react";

import { useChangesStore } from "../stores/changesStore";
import { usePaletteStore } from "../stores/paletteStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";

import type { CenterTab } from "../stores/uiStore";

interface Args {
  setTab: (tab: CenterTab) => void;
}

/// Global keyboard shortcuts. Most fire CustomEvents so the relevant component
/// (Terminal, AgentChat) decides what to do — keeps coupling minimal.
export function useKeyboardShortcuts({ setTab }: Args) {
  useEffect(() => {
    const newSession = () => {
      const project = useSessionStore.getState().project;
      if (!project) return;
      const terminals = useTerminalsStore.getState();
      terminals.add(project.root);
      setTab("terminal");
      void terminals.persist();
    };

    const cycleSession = (direction: 1 | -1): boolean => {
      const terminals = useTerminalsStore.getState();
      const live = terminals.sessions.filter((s) => s.status === "live");
      if (live.length === 0) return false;
      const current = live.findIndex((s) => s.id === terminals.activeId);
      const idx = current >= 0 ? current : 0;
      const next = live[(idx + direction + live.length) % live.length];
      terminals.setActive(next.id);
      setTab("terminal");
      return true;
    };

    const closeActiveSession = () => {
      const terminals = useTerminalsStore.getState();
      const active = terminals.sessions.find((s) => s.id === terminals.activeId);
      if (!active) return;
      if (active.status === "live" && !confirm(`Close session "${active.name}"? Process will be killed.`)) return;
      void terminals.close(active.id);
    };

    const handler = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      const target = e.target as HTMLElement | null;
      const inTerminal = !!target?.closest(".terminal-host");
      const inField = !!target && !inTerminal && (target.tagName === "INPUT" || target.tagName === "TEXTAREA");

      if (e.key === "Tab") {
        if (cycleSession(e.shiftKey ? -1 : 1)) e.preventDefault();
        return;
      }

      switch (e.key) {
        case "p":
        case "P":
          usePaletteStore.getState().openPalette();
          e.preventDefault();
          break;
        case "1":
          setTab("terminal");
          e.preventDefault();
          break;
        case "2":
          setTab("changes");
          e.preventDefault();
          break;
        case "3":
          setTab("preview");
          e.preventDefault();
          break;
        case "b":
        case "B":
          window.dispatchEvent(new CustomEvent("ac:toggle-sidebar"));
          e.preventDefault();
          break;
        case "j":
        case "J":
          // Ctrl+J is line-feed (LF) inside the terminal — leave it to the PTY.
          if (inTerminal) return;
          window.dispatchEvent(new CustomEvent("ac:toggle-right-panel"));
          e.preventDefault();
          break;
        case "t":
        case "T":
          // Ctrl+T is readline transpose-chars inside the terminal — don't steal it.
          if (inTerminal || inField) return;
          newSession();
          e.preventDefault();
          break;
        case "e":
        case "E":
          // Ctrl+E is readline end-of-line inside the terminal — don't steal it
          // there; anywhere else it toggles the prompt composer.
          if (inTerminal || inField) return;
          window.dispatchEvent(new CustomEvent("ac:toggle-composer"));
          e.preventDefault();
          break;
        case "]":
          // Ctrl+] is a terminal control sequence — leave it to the PTY.
          if (inTerminal) return;
          if (cycleSession(1)) e.preventDefault();
          break;
        case "[":
          // Ctrl+[ is ESC inside the terminal (vim, readline) — never intercept it there.
          if (inTerminal) return;
          if (cycleSession(-1)) e.preventDefault();
          break;
        case "w":
        case "W":
          // Ctrl+W is readline delete-previous-word inside the terminal — don't steal it.
          if (inTerminal || inField) return;
          closeActiveSession();
          e.preventDefault();
          break;
        case "/":
          window.dispatchEvent(new CustomEvent("ac:open-shortcuts"));
          e.preventDefault();
          break;
        case "k":
        case "K":
          window.dispatchEvent(new CustomEvent("ac:focus-chat"));
          e.preventDefault();
          break;
        case "l":
        case "L":
          // Only intercept when focus is not inside the terminal (so bash's Ctrl+L still works there).
          if (!target?.closest(".terminal-host")) {
            window.dispatchEvent(new CustomEvent("ac:clear-terminal"));
            e.preventDefault();
          }
          break;
        case "r":
        case "R":
          // Don't preempt browser refresh in dev — only when not in an input field.
          if (inField) return;
          useChangesStore.getState().refresh();
          e.preventDefault();
          break;
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [setTab]);
}
