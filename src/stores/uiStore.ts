import { create } from "zustand";

export type CenterTab = "terminal" | "changes" | "preview";

interface UIState {
  tab: CenterTab;
  setTab: (t: CenterTab) => void;
}

export const useUIStore = create<UIState>((set) => ({
  tab: "terminal",
  setTab: (tab) => set({ tab }),
}));
