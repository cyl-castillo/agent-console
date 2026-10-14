import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/// D13: an agent said no, or asked for a human — through the console's MCP
/// `decline` tool. Each one is a decision for the Inbox: acknowledged by the
/// human, never retried by the console. Kept per project, locally.
export interface Decline {
  id: string;
  seq: number;
  ts: number;
  caller: string;
  kind: "decline" | "escalate";
  reason: string;
  suggestion?: string;
  session?: string;
  project: string;
  resolved: boolean;
}

const KEY = "agent-console:declines";

interface DeclinesState {
  items: Decline[];
  add: (d: Omit<Decline, "id" | "resolved">) => void;
  resolve: (id: string) => void;
}

function load(): Decline[] {
  try {
    if (typeof localStorage === "undefined") return [];
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Decline[]) : [];
  } catch {
    return [];
  }
}

function persist(items: Decline[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(items.slice(-200)));
  } catch {
    /* ignore */
  }
}

export const useDeclinesStore = create<DeclinesState>((set, get) => ({
  items: load(),
  add: (d) => {
    const id = `decline-${d.project}-${d.seq}`;
    if (get().items.some((x) => x.id === id)) return;
    const items = [...get().items, { ...d, id, resolved: false }];
    set({ items });
    persist(items);
  },
  resolve: (id) => {
    const items = get().items.map((x) => (x.id === id ? { ...x, resolved: true } : x));
    set({ items });
    persist(items);
  },
}));

export async function attachDeclineListener(): Promise<UnlistenFn> {
  return listen<Omit<Decline, "id" | "resolved">>("console://declined", (e) => {
    useDeclinesStore.getState().add(e.payload);
  });
}
