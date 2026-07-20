import { useEffect, useMemo, useState } from "react";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import { useChangesStore } from "../stores/changesStore";
import { useSessionStore } from "../stores/sessionStore";
import { ipc } from "../ipc/tauri";
import type { GitCommitInfo, GitFileChange } from "../types/domain";

export function FileInspector() {
  const status = useChangesStore((s) => s.status);
  const selected = useChangesStore((s) => s.selected);
  const diff = useChangesStore((s) => s.diff);
  const project = useSessionStore((s) => s.project);
  const [log, setLog] = useState<GitCommitInfo[]>([]);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);

  const change = useMemo<GitFileChange | undefined>(
    () => status?.changes.find((c) => c.path === selected),
    [status, selected],
  );

  const stats = useMemo(() => countDiffStats(diff), [diff]);

  // Fetch file log when selection changes.
  useEffect(() => {
    setLog([]);
    setLogError(null);
    if (!selected || !status?.isRepo) return;
    setLogLoading(true);
    ipc
      .gitFileLog(selected, 5)
      .then((rows) => setLog(rows))
      .catch((e) => setLogError(String(e)))
      .finally(() => setLogLoading(false));
  }, [selected, status?.isRepo]);

  const absPath = project && selected ? joinPath(project.root, selected) : null;

  if (!status) {
    return (
      <InspectorShell>
        <div className="placeholder">Loading…</div>
      </InspectorShell>
    );
  }
  if (!status.isRepo) {
    return (
      <InspectorShell>
        <div className="placeholder" style={{ padding: 16 }}>
          Not a git repository.
        </div>
      </InspectorShell>
    );
  }
  if (!selected || !change) {
    return (
      <InspectorShell>
        <div className="placeholder" style={{ padding: 16, fontSize: 12 }}>
          Select a file from the Changes list to see its details.
        </div>
      </InspectorShell>
    );
  }

  const onOpen = async () => {
    if (!absPath) return;
    try {
      await openPath(absPath);
    } catch (e) {
      alert(`Could not open file: ${e}`);
    }
  };

  const onReveal = async () => {
    if (!absPath) return;
    try {
      await revealItemInDir(absPath);
    } catch {
      /* best-effort */
    }
  };

  return (
    <InspectorShell>
      <section className="inspector-section">
        <div className="inspector-label">file</div>
        <div className="inspector-path" title={absPath ?? selected}>
          {selected}
        </div>
        {absPath && <div className="inspector-abspath">{absPath}</div>}
      </section>

      <section className="inspector-section">
        <div className="inspector-row">
          <div>
            <div className="inspector-label">status</div>
            <div className="inspector-status">
              <span className="inspector-code">{change.untracked ? "??" : change.code}</span>
              <span className="inspector-status-text">{describeStatus(change)}</span>
            </div>
          </div>
          {(stats.added > 0 || stats.deleted > 0) && (
            <div>
              <div className="inspector-label">changes</div>
              <div className="inspector-stats">
                <span className="inspector-plus">+{stats.added}</span>
                <span className="inspector-minus">−{stats.deleted}</span>
              </div>
            </div>
          )}
        </div>
      </section>

      <section className="inspector-section">
        <div className="inspector-actions">
          <button onClick={onOpen} disabled={!absPath} title="Open with system default app">
            open in editor
          </button>
          <button onClick={onReveal} disabled={!absPath} title="Reveal in file manager">
            reveal
          </button>
        </div>
      </section>

      <section className="inspector-section">
        <div className="inspector-label">recent commits</div>
        {logLoading && (
          <div className="placeholder" style={{ padding: 0, fontSize: 11 }}>
            Loading…
          </div>
        )}
        {!logLoading && logError && (
          <div className="placeholder" style={{ padding: 0, fontSize: 11, color: "var(--danger)" }}>
            {logError}
          </div>
        )}
        {!logLoading && !logError && log.length === 0 && (
          <div className="placeholder" style={{ padding: 0, fontSize: 11 }}>
            No history yet (untracked or new file).
          </div>
        )}
        {!logLoading && !logError && log.length > 0 && (
          <ul className="inspector-log">
            {log.map((c) => (
              <li key={c.sha} className="inspector-log-row" title={`${c.sha}\n${c.author}`}>
                <span className="inspector-sha">{c.shortSha}</span>
                <span className="inspector-subject">{c.subject}</span>
                <span className="inspector-date">{formatDate(c.dateMs)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </InspectorShell>
  );
}

function InspectorShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="workbench">
      <div className="workbench-header">
        <span>file inspector</span>
      </div>
      <div className="workbench-body">{children}</div>
    </div>
  );
}

function countDiffStats(diff: string): { added: number; deleted: number } {
  if (!diff) return { added: 0, deleted: 0 };
  let added = 0;
  let deleted = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++") || line.startsWith("---")) continue;
    if (line.startsWith("+")) added++;
    else if (line.startsWith("-")) deleted++;
  }
  return { added, deleted };
}

function describeStatus(c: GitFileChange): string {
  if (c.untracked) return "untracked";
  const parts: string[] = [];
  if (c.staged) parts.push("staged");
  if (c.unstaged) parts.push("unstaged");
  return parts.join(" + ") || "unchanged";
}

function joinPath(root: string, rel: string): string {
  if (rel.startsWith("/")) return rel;
  const sep = root.endsWith("/") ? "" : "/";
  return `${root}${sep}${rel}`;
}

function formatDate(ms: number): string {
  if (!ms) return "";
  const d = new Date(ms);
  const now = Date.now();
  const diff = (now - ms) / 1000;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  if (diff < 7 * 86400) return `${Math.floor(diff / 86400)}d`;
  return d.toLocaleDateString();
}
