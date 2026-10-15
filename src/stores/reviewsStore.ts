import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ReviewVerdict } from "../types/domain";

/// D10: verdicts of the reviewer agent, one decision each for the Inbox.
/// The verdict itself is in the ledger; this is the human's queue of
/// answers (accept the block / override it with a reason). Per project,
/// locally.
export interface Review {
  id: string;
  ts: number;
  project: string;
  verdict: ReviewVerdict;
  resolved: boolean;
}

const KEY = "agent-console:reviews";

interface ReviewsState {
  items: Review[];
  add: (project: string, verdict: ReviewVerdict, ts?: number) => void;
  resolve: (id: string) => void;
}

function load(): Review[] {
  try {
    if (typeof localStorage === "undefined") return [];
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Review[]) : [];
  } catch {
    return [];
  }
}

function persist(items: Review[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(items.slice(-200)));
  } catch {
    /* ignore */
  }
}

export const useReviewsStore = create<ReviewsState>((set, get) => ({
  items: load(),
  add: (project, verdict, ts = Date.now()) => {
    const id = `review-${project}-${verdict.turnId}-${verdict.seq ?? ts}`;
    if (get().items.some((x) => x.id === id)) return;
    const items = [...get().items, { id, ts, project, verdict, resolved: false }];
    set({ items });
    persist(items);
  },
  resolve: (id) => {
    const items = get().items.map((x) => (x.id === id ? { ...x, resolved: true } : x));
    set({ items });
    persist(items);
  },
}));

export async function attachReviewListener(): Promise<UnlistenFn> {
  return listen<{ project: string; verdict: ReviewVerdict }>("console://review", (e) => {
    useReviewsStore.getState().add(e.payload.project, e.payload.verdict);
  });
}
