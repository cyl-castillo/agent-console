import { create } from "zustand";

export type CenterTab = "terminal" | "changes" | "preview";

/// Shell-level UI state: which center tab is up, which side panels are open,
/// whether the composer is showing. Lives in a store (not App state) so the
/// palette, voice commands and the extracted shell pieces share one truth.
interface UIState {
  tab: CenterTab;
  setTab: (t: CenterTab) => void;
  leftOpen: boolean;
  rightOpen: boolean;
  composerOpen: boolean;
  setLeftOpen: (v: boolean) => void;
  setRightOpen: (v: boolean) => void;
  toggleLeft: () => void;
  toggleRight: () => void;
  setComposerOpen: (v: boolean) => void;
  /// Palette/shortcut toggle: always lands on the terminal tab first.
  toggleComposer: () => void;
}

export const useUIStore = create<UIState>((set) => ({
  tab: "terminal",
  setTab: (tab) => set({ tab }),
  leftOpen: false,
  rightOpen: true,
  composerOpen: false,
  setLeftOpen: (leftOpen) => set({ leftOpen }),
  setRightOpen: (rightOpen) => set({ rightOpen }),
  toggleLeft: () => set((s) => ({ leftOpen: !s.leftOpen })),
  toggleRight: () => set((s) => ({ rightOpen: !s.rightOpen })),
  setComposerOpen: (composerOpen) => set({ composerOpen }),
  toggleComposer: () => set((s) => ({ tab: "terminal", composerOpen: !s.composerOpen })),
}));
