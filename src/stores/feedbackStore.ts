import { create } from "zustand";

import { ipc } from "../ipc/tauri";
import type {
  FeedbackCategory,
  FeedbackContext,
  FeedbackInput,
  FeedbackSeverity,
} from "../types/domain";

interface FeedbackState {
  devEnabled: boolean | null;
  ctx: FeedbackContext | null;
  title: string;
  description: string;
  category: FeedbackCategory;
  severity: FeedbackSeverity;
  status: "idle" | "submitting" | "success" | "error";
  error: string | null;
  lastUrl: string | null;

  init: () => Promise<void>;
  refreshContext: () => Promise<void>;
  setField: (patch: Partial<Pick<FeedbackState, "title" | "description" | "category" | "severity">>) => void;
  reset: () => void;
  submit: () => Promise<void>;
}

export const useFeedbackStore = create<FeedbackState>((set, get) => ({
  devEnabled: null,
  ctx: null,
  title: "",
  description: "",
  category: "bug",
  severity: "medium",
  status: "idle",
  error: null,
  lastUrl: null,

  init: async () => {
    try {
      const enabled = await ipc.feedbackDevEnabled();
      set({ devEnabled: enabled });
      if (enabled) {
        const ctx = await ipc.feedbackContext();
        set({ ctx });
      }
    } catch (e) {
      set({ devEnabled: false, error: String(e) });
    }
  },

  refreshContext: async () => {
    if (!get().devEnabled) return;
    try { set({ ctx: await ipc.feedbackContext() }); } catch { /* ignore */ }
  },

  setField: (patch) => set(patch),

  reset: () => set({
    title: "", description: "", category: "bug", severity: "medium",
    status: "idle", error: null, lastUrl: null,
  }),

  submit: async () => {
    const { title, description, category, severity } = get();
    if (!title.trim() || !description.trim()) {
      set({ status: "error", error: "Title and description are required." });
      return;
    }
    set({ status: "submitting", error: null, lastUrl: null });
    const input: FeedbackInput = {
      title: title.trim(),
      description: description.trim(),
      category,
      severity,
    };
    try {
      const url = await ipc.feedbackSubmit(input);
      set({
        status: "success", lastUrl: url,
        title: "", description: "",
      });
    } catch (e) {
      set({ status: "error", error: String(e) });
    }
  },
}));
