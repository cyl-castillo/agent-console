import { useChangesStore } from "../stores/changesStore";
import { useUIStore } from "../stores/uiStore";
import type { GitFileChange } from "../types/domain";

export function ChangesList() {
  const status = useChangesStore((s) => s.status);
  const selected = useChangesStore((s) => s.selected);
  const setSelected = useChangesStore((s) => s.setSelected);
  const refresh = useChangesStore((s) => s.refresh);
  const setTab = useUIStore((s) => s.setTab);

  const changes = status?.changes ?? [];

  const onPick = (path: string) => {
    setSelected(path);
    setTab("changes");
  };

  if (!status) {
    return <div className="placeholder" style={{ padding: "8px 12px" }}>Loading…</div>;
  }

  if (!status.isRepo) {
    return <div className="placeholder" style={{ padding: "8px 12px" }}>Not a git repo.</div>;
  }

  if (changes.length === 0) {
    return (
      <div className="changes-empty">
        <span>Working tree clean.</span>
        <button className="btn btn-link" onClick={() => refresh()} title="Refresh">refresh</button>
      </div>
    );
  }

  return (
    <ul className="changes-list">
      {changes.map((c) => (
        <ChangeRow
          key={c.path}
          change={c}
          active={c.path === selected}
          onPick={() => onPick(c.path)}
        />
      ))}
    </ul>
  );
}

function ChangeRow({ change, active, onPick }: {
  change: GitFileChange;
  active: boolean;
  onPick: () => void;
}) {
  const { tag, kind } = describe(change);
  const idx = change.path.lastIndexOf("/");
  const dir = idx >= 0 ? change.path.slice(0, idx) : "";
  const name = idx >= 0 ? change.path.slice(idx + 1) : change.path;

  return (
    <li
      className={`change-row ${active ? "active" : ""}`}
      onClick={onPick}
      title={change.path}
    >
      <span className={`change-tag ${kind}`}>{tag}</span>
      <span className="change-name">{name}</span>
      {dir && <span className="change-dir">{dir}</span>}
    </li>
  );
}

function describe(c: GitFileChange): { tag: string; kind: string } {
  if (c.untracked) return { tag: "U", kind: "untracked" };
  const code = c.code?.trim();
  if (code === "A" || code?.startsWith("A")) return { tag: "A", kind: "added" };
  if (code === "D" || code?.startsWith("D")) return { tag: "D", kind: "deleted" };
  if (code === "R" || code?.startsWith("R")) return { tag: "R", kind: "renamed" };
  return { tag: "M", kind: "modified" };
}
