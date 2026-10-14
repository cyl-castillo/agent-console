import { FileInspector } from "../components/FileInspector";
import { WorkbenchSubTabs, WorkbenchTabs } from "../components/WorkbenchTabs";
import { useUIStore } from "../stores/uiStore";
import { useWorkbenchStore } from "../stores/workbenchStore";
import { WorkbenchPanel } from "./panels";

/// The right column (v1 shell). On the Changes tab it becomes the file
/// inspector; otherwise the workbench tabs and the active panel.
export function Workbench() {
  const centerTab = useUIStore((s) => s.tab);
  const tab = useWorkbenchStore((s) => s.tab);
  const setTab = useWorkbenchStore((s) => s.setTab);

  return (
    <aside className="panel right">
      {centerTab === "changes" ? (
        <FileInspector />
      ) : (
        <div className="workbench-layout">
          <WorkbenchTabs active={tab} onChange={setTab} />
          <div className="workbench-content">
            <WorkbenchSubTabs active={tab} onChange={setTab} />
            <WorkbenchPanel tab={tab} />
          </div>
        </div>
      )}
    </aside>
  );
}
