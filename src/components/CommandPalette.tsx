import { useEffect, useRef } from "react";

import { useChangesStore } from "../stores/changesStore";
import { usePaletteStore } from "../stores/paletteStore";
import { useTerminalsStore } from "../stores/terminalsStore";

const KIND_ICON: Record<string, string> = {
  file: "📄",
  action: "▸",
  session: "▮",
  branch: "⎇",
};

export function CommandPalette() {
  const open = usePaletteStore((s) => s.open);
  const query = usePaletteStore((s) => s.query);
  const setQuery = usePaletteStore((s) => s.setQuery);
  const close = usePaletteStore((s) => s.close);
  const selectedIndex = usePaletteStore((s) => s.selectedIndex);
  const setSelectedIndex = usePaletteStore((s) => s.setSelectedIndex);
  const execute = usePaletteStore((s) => s.execute);
  const filesLoading = usePaletteStore((s) => s.filesLoading);
  const filesError = usePaletteStore((s) => s.filesError);
  const pendingBranchSwitch = usePaletteStore((s) => s.pendingBranchSwitch);
  // Subscribe to inputs of results() so React recomputes on change.
  usePaletteStore((s) => s.files);
  useTerminalsStore((s) => s.sessions);
  useTerminalsStore((s) => s.activeId);
  useChangesStore((s) => s.branches);
  useChangesStore((s) => s.status);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const items = usePaletteStore.getState().results();

  useEffect(() => {
    if (open) {
      // small delay to let CSS show the modal before focusing
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const node = listRef.current?.querySelector<HTMLElement>(`[data-index="${selectedIndex}"]`);
    node?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex, open]);

  if (!open) return null;

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex(Math.min(items.length - 1, selectedIndex + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex(Math.max(0, selectedIndex - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = items[selectedIndex];
      if (item) void execute(item);
    }
  };

  const placeholder = query.startsWith(">")
    ? "Type an action…"
    : query.startsWith(":")
      ? "Search open sessions…"
      : query.startsWith("@")
        ? "Search branches…"
        : "Search files & actions  ( >  actions   :  sessions   @  branches )";

  return (
    <div className="palette-backdrop" onMouseDown={close}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          className="palette-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          spellCheck={false}
        />
        <div className="palette-list" ref={listRef}>
          {filesLoading && items.length === 0 && (
            <div className="palette-empty">Indexing files…</div>
          )}
          {filesError && (
            <div className="palette-empty palette-error">Index error: {filesError}</div>
          )}
          {!filesLoading && items.length === 0 && (
            <div className="palette-empty">No matches</div>
          )}
          {items.map((it, idx) => {
            const isSelected = idx === selectedIndex;
            const isPending = pendingBranchSwitch === it.id;
            return (
              <div
                key={it.id}
                data-index={idx}
                className={`palette-row ${isSelected ? "selected" : ""} kind-${it.kind} ${isPending ? "pending" : ""}`}
                onMouseEnter={() => setSelectedIndex(idx)}
                onClick={() => void execute(it)}
              >
                <span className={`palette-icon kind-${it.kind}`}>{KIND_ICON[it.kind] ?? "·"}</span>
                <span className="palette-label">{it.label}</span>
                {it.hint && <span className="palette-hint">{it.hint}</span>}
                {it.badge && <span className="palette-badge">{it.badge}</span>}
                {isPending && it.warn && (
                  <span className="palette-confirm">{it.warn}</span>
                )}
              </div>
            );
          })}
        </div>
        <div className="palette-footer">
          <span><kbd>↑</kbd><kbd>↓</kbd> move</span>
          <span><kbd>Enter</kbd> run</span>
          <span><kbd>Esc</kbd> close</span>
          <span className="spacer" />
          <span><kbd>&gt;</kbd> actions · <kbd>:</kbd> sessions · <kbd>@</kbd> branches</span>
        </div>
      </div>
    </div>
  );
}
