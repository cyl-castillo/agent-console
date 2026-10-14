import { create } from "zustand";

export type Overlay = "about" | "gettingStarted" | "shortcuts" | "modules" | "wizard";

/// Which app-level modals are open. The palette and voice commands open them
/// through window events (kept for compatibility); the shell reads the store.
interface OverlaysState {
  open: Record<Overlay, boolean>;
  show: (o: Overlay) => void;
  hide: (o: Overlay) => void;
}

const none: Record<Overlay, boolean> = {
  about: false,
  gettingStarted: false,
  shortcuts: false,
  modules: false,
  wizard: false,
};

export const useOverlaysStore = create<OverlaysState>((set) => ({
  open: { ...none },
  show: (o) => set((s) => ({ open: { ...s.open, [o]: true } })),
  hide: (o) => set((s) => ({ open: { ...s.open, [o]: false } })),
}));

const EVENTS: [string, Overlay][] = [
  ["ac:open-modules", "modules"],
  ["ac:open-getting-started", "gettingStarted"],
  ["ac:open-shortcuts", "shortcuts"],
  ["ac:open-wizard", "wizard"],
];

export function attachOverlayListeners(): () => void {
  const offs = EVENTS.map(([name, overlay]) => {
    const h = () => useOverlaysStore.getState().show(overlay);
    window.addEventListener(name, h);
    return () => window.removeEventListener(name, h);
  });
  return () => offs.forEach((f) => f());
}
