import { useEffect } from "react";
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

interface Props extends ProjectBootstrap {
  project: Project;
}

/// The redesigned shell (flag ui.v2): rail → list → centre → inspector.
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
  const { decisions } = useInbox();
  const sessions = useTerminalsStore((s) => s.sessions);
  const toggleList = useShellStore((s) => s.toggleList);
  const loadJobs = useJobsStore((s) => s.load);
  const initJobListeners = useJobsStore((s) => s.initListeners);

  useEffect(() => attachShellListeners(), []);
  useBudgetWatch();
  // Ctrl+` toggles the terminal drawer; the shortcut hook dispatches it.
  useEffect(() => {
    const h = () => useShellStore.getState().toggleDrawer();
    window.addEventListener("ac:toggle-terminal-drawer", h);
    return () => window.removeEventListener("ac:toggle-terminal-drawer", h);
  }, []);
  // The top bar's Workspace button and the palette's "toggle sidebar" still
  // flip uiStore.leftOpen; in this shell that gesture means the list pane.
  useEffect(
    () =>
      useUIStore.subscribe((s, prev) => {
        if (s.leftOpen !== prev.leftOpen) toggleList();
      }),
    [toggleList],
  );
  // The inbox counts jobs that need a human; fetch the board once per project.
  useEffect(() => {
    void initJobListeners();
    void loadJobs();
  }, [project.root, loadJobs, initJobListeners]);

  const counts = {
    inbox: decisions,
    sessions: sessions.filter((s) => s.status === "live").length,
  };
  const hasList = destination !== "inbox" && listOpen;
  const hasInspector = rightOpen && inspectorFor(destination, centerTab) !== null;

  return (
    <TooltipProvider>
      <div
        className={cn(
          "grid h-full w-full bg-background text-foreground",
          "grid-rows-[32px_minmax(0,1fr)_24px]",
          "[grid-template-areas:'top_top_top_top'_'rail_list_center_inspector'_'status_status_status_status']",
        )}
        style={{
          gridTemplateColumns: `56px ${hasList ? "240px" : "0px"} minmax(0,1fr) ${hasInspector ? "360px" : "0px"}`,
        }}
        data-testid="shell-v2"
        data-destination={destination}
      >
        <div className="[grid-area:top] min-w-0">
          <TopBar project={project} />
        </div>
        <div className="[grid-area:rail] min-h-0">
          <Rail counts={counts} />
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
      </div>
    </TooltipProvider>
  );
}
