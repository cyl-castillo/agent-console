import { useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";

const PANEL_W_KEY = "agent-console:panel-width:";
const DEFAULT_W = { left: 240, right: 320 } as const;
export type Side = "left" | "right";

function loadPanelWidth(side: Side): number {
  try {
    const v = Number(localStorage.getItem(PANEL_W_KEY + side));
    if (Number.isFinite(v) && v > 0) return v;
  } catch {
    /* ignore */
  }
  return DEFAULT_W[side];
}

function savePanelWidth(side: Side, w: number) {
  try {
    localStorage.setItem(PANEL_W_KEY + side, String(w));
  } catch {
    /* ignore */
  }
}

function clampW(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, Math.round(v)));
}

/// Drag-to-resize for the two side panels. Widths are written straight to the
/// shell's CSS variables during the drag (no React re-render per pixel) and
/// committed to state + localStorage on pointer-up.
export function usePanelResize(appRef: RefObject<HTMLDivElement | null>) {
  const [leftW, setLeftW] = useState<number>(() => loadPanelWidth("left"));
  const [rightW, setRightW] = useState<number>(() => loadPanelWidth("right"));
  const dragSide = useRef<null | Side>(null);
  const pendingW = useRef<number | null>(null);

  const startResize = (side: Side) => (e: ReactPointerEvent) => {
    e.preventDefault();
    dragSide.current = side;
    pendingW.current = null;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    el.classList.add("dragging");
  };
  const onResizeMove = (e: ReactPointerEvent) => {
    const side = dragSide.current;
    const app = appRef.current;
    if (!side || !app) return;
    const rect = app.getBoundingClientRect();
    const cap = Math.round(rect.width * 0.45); // keep the center usable
    let w: number;
    if (side === "left") {
      w = clampW(e.clientX - rect.left, 180, Math.min(480, cap));
      app.style.setProperty("--left-w", `${w}px`);
    } else {
      w = clampW(rect.right - e.clientX, 240, Math.min(600, cap));
      app.style.setProperty("--right-w", `${w}px`);
    }
    pendingW.current = w; // committed to React state on pointer-up
  };
  const endResize = (e: ReactPointerEvent) => {
    const side = dragSide.current;
    if (!side) return;
    const el = e.currentTarget as HTMLElement;
    el.classList.remove("dragging");
    el.releasePointerCapture?.(e.pointerId);
    dragSide.current = null;
    const w = pendingW.current;
    pendingW.current = null;
    if (w == null) return;
    if (side === "left") {
      setLeftW(w);
      savePanelWidth("left", w);
    } else {
      setRightW(w);
      savePanelWidth("right", w);
    }
  };
  const resetResize = (side: Side) => () => {
    const w = DEFAULT_W[side];
    if (side === "left") setLeftW(w);
    else setRightW(w);
    savePanelWidth(side, w);
  };

  return { leftW, rightW, startResize, onResizeMove, endResize, resetResize };
}
