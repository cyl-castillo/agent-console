import { useRef, type CSSProperties } from "react";
import type { Project } from "../types/domain";
import { LeftSidebar } from "../components/LeftSidebar";
import { StatusBar } from "../components/StatusBar";
import { useRoundtableStore } from "../stores/roundtableStore";
import { useUIStore } from "../stores/uiStore";
import { useWorkbenchStore } from "../stores/workbenchStore";
import { CenterPane } from "./CenterPane";
import { TopBar } from "./TopBar";
import { Workbench } from "./Workbench";
import { usePanelResize } from "./usePanelResize";
import type { ProjectBootstrap } from "./useProjectBootstrap";

interface Props extends ProjectBootstrap {
  project: Project;
}

/// The three-column shell: top bar, left sidebar, center pane, workbench,
/// status bar, plus the drag handles between the columns.
export function Shell({ project, workspace, liveTerminals, activeTerminal, activeStopped }: Props) {
  const leftOpen = useUIStore((s) => s.leftOpen);
  const rightOpen = useUIStore((s) => s.rightOpen);
  const appRef = useRef<HTMLDivElement>(null);
  const { leftW, rightW, startResize, onResizeMove, endResize, resetResize } =
    usePanelResize(appRef);
  const openRoom = useRoundtableStore((s) => s.openRoom);
  const setWorkbenchTab = useWorkbenchStore((s) => s.setTab);
  // Open a saved room read-only and surface the room tab so it's visible.
  const onOpenRoom = (id: string) => {
    void openRoom(id);
    setWorkbenchTab("roundtable");
  };

  return (
    <div
      ref={appRef}
      className={`app ${leftOpen ? "" : "left-collapsed"} ${rightOpen ? "" : "right-collapsed"}`}
      style={{ "--left-w": `${leftW}px`, "--right-w": `${rightW}px` } as CSSProperties}
    >
      <TopBar project={project} />

      {leftOpen && <LeftSidebar onOpenRoom={onOpenRoom} />}

      <CenterPane
        project={project}
        liveTerminals={liveTerminals}
        activeTerminal={activeTerminal}
        activeStopped={activeStopped}
      />

      <Workbench />
      <StatusBar workspace={workspace} />

      {leftOpen && (
        <div
          className="resize-handle left"
          onPointerDown={startResize("left")}
          onPointerMove={onResizeMove}
          onPointerUp={endResize}
          onDoubleClick={resetResize("left")}
          title="Drag to resize · double-click to reset"
        />
      )}
      {rightOpen && (
        <div
          className="resize-handle right"
          onPointerDown={startResize("right")}
          onPointerMove={onResizeMove}
          onPointerUp={endResize}
          onDoubleClick={resetResize("right")}
          title="Drag to resize · double-click to reset"
        />
      )}
    </div>
  );
}
