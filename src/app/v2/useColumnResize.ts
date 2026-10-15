import { useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { useShellStore } from "../../stores/shellStore";

export type Column = "list" | "inspector";

/// Drag-to-resize for the list pane and the inspector. During the drag the
/// width goes straight to a CSS variable on the shell (no React render per
/// pixel); on pointer-up it is committed to the shell store (persisted) —
/// the v1 handles, on the v2 columns. Double-click resets to the default.
export function useColumnResize(shellRef: React.RefObject<HTMLDivElement | null>) {
  const setWidth = useShellStore((s) => s.setWidth);
  const drag = useRef<Column | null>(null);
  const pending = useRef<number | null>(null);
  const [dragging, setDragging] = useState<Column | null>(null);

  const start = (col: Column) => (e: ReactPointerEvent<HTMLElement>) => {
    e.preventDefault();
    drag.current = col;
    pending.current = null;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging(col);
  };
  const move = (e: ReactPointerEvent<HTMLElement>) => {
    const col = drag.current;
    const shell = shellRef.current;
    if (!col || !shell) return;
    const rect = shell.getBoundingClientRect();
    // The list starts after the 56 px rail; the inspector ends at the right edge.
    const w = col === "list" ? e.clientX - rect.left - 56 : rect.right - e.clientX;
    pending.current = w;
    shell.style.setProperty(col === "list" ? "--list-w" : "--inspector-w", `${Math.round(w)}px`);
  };
  const end = (e: ReactPointerEvent<HTMLElement>) => {
    const col = drag.current;
    if (!col) return;
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    drag.current = null;
    setDragging(null);
    const shell = shellRef.current;
    shell?.style.removeProperty("--list-w");
    shell?.style.removeProperty("--inspector-w");
    if (pending.current !== null) setWidth(col, pending.current);
    pending.current = null;
  };
  const reset = (col: Column) => () => setWidth(col, null);

  return { start, move, end, reset, dragging };
}
