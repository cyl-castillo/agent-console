import { useEffect, useRef, useState } from "react";
import type { Project } from "../../types/domain";
import { useJobsStore } from "../../stores/jobsStore";
import { attachShellListeners, useShellStore } from "../../stores/shellStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useUIStore } from "../../stores/uiStore";
import { cn } from "@/lib/cn";
import { TooltipProvider } from "../../ui/Tooltip";
import { TopBar } from "../TopBar";
import type { ProjectBootstrap } from "../useProjectBootstrap";
import { CenterV2 } from "./CenterV2";
import { Inspector, inspectorFor } from "./Inspector";
import { ListPane } from "./ListPane";
import { Rail } from "./Rail";
import { StatusBarV2 } from "./StatusBarV2";
import { useInbox } from "./useInbox";
import { useBudgetWatch } from "./useBudgetWatch";
import { useColumnResize, type Column } from "./useColumnResize";
import { NARROW_QUERY, useMediaQuery } from "../../hooks/useMediaQuery";

interface Props extends ProjectBootstrap {
  project: Project;
}

/// The shell: rail → list → centre → inspector.
/// The decision queue is the primary view; the terminal lives under
/// Sessions; every panel keeps working from its new home.
export function ShellV2({
  project,
  workspace,
  liveTerminals,
  activeTerminal,
  activeStopped,
}: Props) {
  const destination = useShellStore((s) => s.destination);
  const listOpen = useShellStore((s) => s.listOpen);
  const rightOpen = useUIStore((s) => s.rightOpen);
  const centerTab = useUIStore((s) => s.tab);
  const { decisions, byState } = useInbox();
  const toggleList = useShellStore((s) => s.toggleList);
  const focus = useShellStore((s) => s.focus);
  const toggleFocus = useShellStore((s) => s.toggleFocus);
  const listWidth = useShellStore((s) => s.listWidth);
  const inspectorWidth = useShellStore((s) => s.inspectorWidth);
  const shellRef = useRef<HTMLDivElement>(null);
  const resize = useColumnResize(shellRef);
  // Below 900 px the side columns collapse on their own; the list can still
  // be summoned (Workspace button, Ctrl+B) and then takes the inspector's
  // place rather than squeezing the centre.
  const narrow = useMediaQuery(NARROW_QUERY);
  const [narrowListOpen, setNarrowListOpen] = useState(false);
  const loadJobs = useJobsStore((s) => s.load);
  const initJobListeners = useJobsStore((s) => s.initListeners);

  useEffect(() => attachShellListeners(), []);
  useBudgetWatch();
  // F4 landing: with nothing to decide and at most one session, Sessions is
  // where the work is — not an empty Inbox. Once per project open, when
  // the sessions have hydrated.
  const ready = useTerminalsStore((s) => s.ready);
  const landed = useRef<string | null>(null);
  useEffect(() => {
    if (!ready || landed.current === project.root) return;
    landed.current = project.root;
    const live = useTerminalsStore.getState().sessions.filter((s) => !s.archived).length;
    if (decisions === 0 && live <= 1 && useShellStore.getState().destination === "inbox") {
      useShellStore.getState().goTo("sessions");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, project.root]);
  // Ctrl+` toggles the terminal drawer; the shortcut hook dispatches it.
  useEffect(() => {
    const h = () => useShellStore.getState().toggleDrawer();
    window.addEventListener("ac:toggle-terminal-drawer", h);
    return () => window.removeEventListener("ac:toggle-terminal-drawer", h);
  }, []);
  // Ctrl+Shift+F / palette "Focus mode": the terminal alone.
  useEffect(() => {
    const h = () => useShellStore.getState().toggleFocus();
    window.addEventListener("ac:toggle-focus", h);
    return () => window.removeEventListener("ac:toggle-focus", h);
  }, []);
  // The top bar's Workspace button and the palette's "toggle sidebar" still
  // flip uiStore.leftOpen; in this shell that gesture means the list pane.
  useEffect(
    () =>
      useUIStore.subscribe((s, prev) => {
        if (s.leftOpen === prev.leftOpen) return;
        if (narrow) setNarrowListOpen((v) => !v);
        else toggleList();
      }),
    [toggleList, narrow],
  );
  // The inbox counts jobs that need a human; fetch the board once per project.
  useEffect(() => {
    void initJobListeners();
    void loadJobs();
  }, [project.root, loadJobs, initJobListeners]);

  // A badge is a call to act: the rail counts the sessions waiting on the
  // human, not the live ones (those are the grey count in the list header).
  const counts = {
    inbox: decisions,
    sessions: byState.get("needs-you")?.length ?? 0,
  };
  const hasList = !focus && destination !== "inbox" && (narrow ? narrowListOpen : listOpen);
  // On a narrow window the list pane is hidden by default and nothing on
  // screen says how to get it: a rail click opens it for that destination
  // (a Windows laptop at 150 % scaling with the window snapped to half the
  // screen is well under 900 CSS px — "only the icons show"). Clicking the
  // active destination again hides it; the Inbox has no list.
  const pickDestination = (d: string) => {
    if (!narrow) return;
    if (d === "inbox") setNarrowListOpen(false);
    else setNarrowListOpen(d === destination ? !narrowListOpen : true);
  };
  const selectedTurn = useShellStore((s) => s.selectedTurnId);
  const hasInspector =
    !focus && rightOpen && !narrow && inspectorFor(destination, centerTab, selectedTurn) !== null;
  const handle = (col: Column, className: string) => (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={col === "list" ? "Resize list pane" : "Resize inspector"}
      data-testid={`resize-${col}`}
      className={cn(
        "absolute bottom-6 top-8 z-5 w-2 cursor-col-resize",
        "after:absolute after:bottom-0 after:left-[3px] after:top-0 after:w-0.5 after:bg-transparent after:content-['']",
        "hover:after:bg-primary",
        resize.dragging === col && "after:bg-primary",
        className,
      )}
      onPointerDown={resize.start(col)}
      onPointerMove={resize.move}
      onPointerUp={resize.end}
      onDoubleClick={resize.reset(col)}
      title="Drag to resize · double-click to reset"
    />
  );

  return (
    <TooltipProvider>
      <div
        ref={shellRef}
        className={cn(
          "relative grid h-full w-full bg-background text-foreground",
          focus ? "grid-rows-[0px_minmax(0,1fr)_24px]" : "grid-rows-[32px_minmax(0,1fr)_24px]",
          "[grid-template-areas:'top_top_top_top'_'rail_list_center_inspector'_'status_status_status_status']",
        )}
        style={{
          ["--list-w-fallback" as string]: `${listWidth}px`,
          ["--inspector-w-fallback" as string]: `${inspectorWidth}px`,
          gridTemplateColumns: `${focus ? "0px" : "56px"} ${hasList ? `var(--list-w, ${listWidth}px)` : "0px"} minmax(0,1fr) ${hasInspector ? `var(--inspector-w, ${inspectorWidth}px)` : "0px"}`,
        }}
        data-testid="shell-v2"
        data-destination={destination}
        data-narrow={narrow || undefined}
        data-focus={focus || undefined}
      >
        <div className={cn("[grid-area:top] min-w-0", focus && "hidden")}>
          <TopBar project={project} />
        </div>
        <div className={cn("[grid-area:rail] min-h-0", focus && "hidden")}>
          <Rail counts={counts} onPick={pickDestination} />
        </div>
        <div
          className={cn(
            "[grid-area:list] min-h-0 overflow-hidden border-r border-border bg-surface-1",
            !hasList && "hidden",
          )}
        >
          <ListPane />
        </div>
        <main className="[grid-area:center] min-h-0 min-w-0 overflow-hidden">
          <CenterV2
            project={project}
            liveTerminals={liveTerminals}
            activeTerminal={activeTerminal}
            activeStopped={activeStopped}
          />
        </main>
        <div
          className={cn("[grid-area:inspector] min-h-0 overflow-hidden", !hasInspector && "hidden")}
        >
          <Inspector />
        </div>
        <div className="[grid-area:status] min-w-0">
          <StatusBarV2 workspace={workspace} />
        </div>
        {focus && (
          <button
            type="button"
            onClick={toggleFocus}
            className="unbutton absolute right-3 top-2 z-10 inline-flex items-center gap-1.5 rounded-pill border border-border bg-surface-2 px-2.5 py-1 font-sans text-caption text-muted-foreground shadow-md hover:bg-[var(--hover)] hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring"
            data-testid="exit-focus"
            title="Leave focus mode (Ctrl+Shift+F)"
          >
            Focus · Ctrl+Shift+F to leave
          </button>
        )}
        {hasList && handle("list", "left-[calc(56px+var(--list-w,var(--list-w-fallback))-4px)]")}
        {hasInspector &&
          handle("inspector", "right-[calc(var(--inspector-w,var(--inspector-w-fallback))-4px)]")}
      </div>
    </TooltipProvider>
  );
}
