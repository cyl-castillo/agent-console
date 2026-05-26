import { create } from "zustand";

const STORAGE_KEY = "agent-console.pinned-skills.v1";

function loadInitial(): Set<string> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return new Set();
    const arr = JSON.parse(raw);
    return Array.isArray(arr) ? new Set(arr) : new Set();
  } catch {
    return new Set();
  }
}

function persist(set: Set<string>) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(Array.from(set)));
  } catch { /* ignore */ }
}

/// Pin key is `${kind}:${source}:${name}` so a project skill and a user skill
/// with the same name can be pinned independently.
export function pinKey(kind: string, source: string, name: string): string {
  return `${kind}:${source}:${name}`;
}

interface PinsState {
  pinned: Set<string>;
  isPinned: (key: string) => boolean;
  toggle: (key: string) => void;
}

export const usePinsStore = create<PinsState>((set, get) => ({
  pinned: loadInitial(),
  isPinned: (key) => get().pinned.has(key),
  toggle: (key) => {
    const next = new Set(get().pinned);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    persist(next);
    set({ pinned: next });
  },
}));
