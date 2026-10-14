import { FileInspector } from "../../components/FileInspector";
import { TurnsPanel } from "../../components/TurnsPanel";
import { useShellStore } from "../../stores/shellStore";
import { useUIStore } from "../../stores/uiStore";

/// The right column, always rotulated: what it shows never changes without a
/// visible title (UX_REVIEW P2.2). Sessions get the turn timeline, or the
/// file inspector while Changes is up; other destinations show nothing here
/// in F2 (the shell collapses the column).
export function inspectorFor(destination: string, centerTab: string): { title: string } | null {
  if (destination !== "sessions") return null;
  return centerTab === "changes" ? { title: "File" } : { title: "Turns" };
}

export function Inspector() {
  const destination = useShellStore((s) => s.destination);
  const centerTab = useUIStore((s) => s.tab);
  const meta = inspectorFor(destination, centerTab);
  if (!meta) return null;
  return (
    <aside
      aria-label={`Inspector: ${meta.title}`}
      data-testid="inspector"
      className="flex h-full min-h-0 flex-col border-l border-border bg-surface-1"
    >
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border px-3 font-sans">
        <span className="text-caption font-semibold uppercase tracking-wide text-faint-foreground">
          Inspector
        </span>
        <span className="text-small font-medium text-foreground">{meta.title}</span>
      </div>
      <div className="workbench-layout min-h-0 flex-1">
        <div className="workbench-content">
          {centerTab === "changes" ? <FileInspector /> : <TurnsPanel />}
        </div>
      </div>
    </aside>
  );
}
