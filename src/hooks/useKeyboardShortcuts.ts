import { useEffect } from "react";

import { useChangesStore } from "../stores/changesStore";

import type { CenterTab } from "../stores/uiStore";

interface Args {
  setTab: (tab: CenterTab) => void;
}

/// Global keyboard shortcuts. Most fire CustomEvents so the relevant component
/// (Terminal, AgentChat) decides what to do — keeps coupling minimal.
export function useKeyboardShortcuts({ setTab }: Args) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      const target = e.target as HTMLElement | null;

      switch (e.key) {
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
          if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
          useChangesStore.getState().refresh();
          e.preventDefault();
          break;
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [setTab]);
}
