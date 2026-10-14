import { create } from "zustand";

/// Feature flags for work that ships dark. `ui.v2` gates the redesigned shell
/// (UI-2026-10.md, phases F1–F6): panels migrate one at a time behind it and
/// the flag is removed when the last one lands. Persisted per machine and
/// mirrored to `<html data-ui>` so CSS can key on it.
export type Flag = "ui.v2";

const STORAGE_KEY = "agent-console:flags";

function load(): Record<Flag, boolean> {
  try {
    if (typeof localStorage === "undefined") return { "ui.v2": false };
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed = raw ? (JSON.parse(raw) as Partial<Record<Flag, boolean>>) : {};
    return { "ui.v2": parsed["ui.v2"] === true };
  } catch {
    return { "ui.v2": false };
  }
}

function apply(flags: Record<Flag, boolean>) {
  if (typeof document === "undefined") return; // store tests run in node
  if (flags["ui.v2"]) document.documentElement.dataset.ui = "v2";
  else delete document.documentElement.dataset.ui;
}

interface FlagsState {
  flags: Record<Flag, boolean>;
  set: (flag: Flag, on: boolean) => void;
  toggle: (flag: Flag) => void;
}

export const useFlagsStore = create<FlagsState>((set, get) => {
  const initial = load();
  apply(initial);
  return {
    flags: initial,
    set: (flag, on) => {
      const flags = { ...get().flags, [flag]: on };
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(flags));
      } catch {
        /* ignore */
      }
      apply(flags);
      set({ flags });
    },
    toggle: (flag) => get().set(flag, !get().flags[flag]),
  };
});
