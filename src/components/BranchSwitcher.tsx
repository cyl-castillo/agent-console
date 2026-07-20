import { useEffect, useMemo, useRef, useState } from "react";

import { useChangesStore } from "../stores/changesStore";
import type { BranchInfo } from "../types/domain";

interface Props {
  currentBranch: string | null;
}

/// Click-to-open popover anchored to the branch chip in the Changes header.
/// Lists local branches with ahead/behind vs upstream; click to checkout.
export function BranchSwitcher({ currentBranch }: Props) {
  const branches = useChangesStore((s) => s.branches);
  const loading = useChangesStore((s) => s.branchesLoading);
  const loadBranches = useChangesStore((s) => s.loadBranches);
  const checkoutBranch = useChangesStore((s) => s.checkoutBranch);

  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [switching, setSwitching] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  // Find ahead/behind for the current branch to render the inline counter.
  const currentInfo = useMemo(() => branches.find((b) => b.current) ?? null, [branches]);

  // Reload list every time the popover opens — branches can change from the
  // terminal (git checkout/branch/delete) and the filesystem watcher already
  // refreshes status, but not this list.
  useEffect(() => {
    if (open) loadBranches();
  }, [open, loadBranches]);

  // Click-outside to dismiss.
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return branches;
    return branches.filter((b) => b.name.toLowerCase().includes(q));
  }, [branches, query]);

  const onPick = async (b: BranchInfo) => {
    if (b.current) {
      setOpen(false);
      return;
    }
    setSwitching(b.name);
    try {
      await checkoutBranch(b.name);
      setOpen(false);
    } catch (e) {
      alert(`Could not switch: ${e}`);
    } finally {
      setSwitching(null);
    }
  };

  return (
    <div className="branch-switcher" ref={wrapRef}>
      <button
        className="branch-chip"
        onClick={() => setOpen((v) => !v)}
        title={currentBranch ? `On ${currentBranch} — click to switch` : "Detached HEAD"}
      >
        <span className="branch-chip-icon">⎇</span>
        <span className="branch-chip-name">{currentBranch ?? "(detached)"}</span>
        {currentInfo && (currentInfo.ahead > 0 || currentInfo.behind > 0) && (
          <span className="branch-chip-ab">
            {currentInfo.ahead > 0 && (
              <span title="commits ahead of upstream">↑{currentInfo.ahead}</span>
            )}
            {currentInfo.behind > 0 && (
              <span title="commits behind upstream">↓{currentInfo.behind}</span>
            )}
          </span>
        )}
        <span className="branch-chip-caret">▾</span>
      </button>

      {open && (
        <div className="branch-popover" role="dialog">
          <div className="branch-search">
            <input
              autoFocus
              className="wb-search-input"
              placeholder="Filter branches…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>

          {loading && branches.length === 0 && <div className="branch-empty">Loading…</div>}
          {!loading && branches.length === 0 && (
            <div className="branch-empty">No local branches.</div>
          )}
          {filtered.length === 0 && branches.length > 0 && (
            <div className="branch-empty">No matches.</div>
          )}

          <ul className="branch-list">
            {filtered.map((b) => (
              <li
                key={b.name}
                className={`branch-item ${b.current ? "current" : ""}`}
                onClick={() => onPick(b)}
                title={b.lastSubject}
              >
                <span className="branch-item-name">
                  {b.current && <span className="branch-current-dot">●</span>}
                  {b.name}
                </span>
                {b.upstream && (
                  <span className="branch-item-upstream" title={`tracking ${b.upstream}`}>
                    {b.ahead > 0 && <span>↑{b.ahead}</span>}
                    {b.behind > 0 && <span>↓{b.behind}</span>}
                    {b.ahead === 0 && b.behind === 0 && <span className="branch-clean">✓</span>}
                  </span>
                )}
                {switching === b.name && <span className="branch-spinner">…</span>}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
