import type { Project } from "../../types/domain";
import type { TerminalSession } from "../../stores/terminalsStore";
import { useShellStore } from "../../stores/shellStore";
import { useWorkbenchStore } from "../../stores/workbenchStore";
import { SessionsCenter } from "./SessionsCenter";
import { WorkbenchPanel } from "../panels";
import { InboxView } from "./InboxView";
import { TAB_LABELS } from "./destinations";

interface Props {
  project: Project;
  liveTerminals: TerminalSession[];
  activeTerminal: TerminalSession | undefined;
  activeStopped: boolean;
}

/// The centre column of the v2 shell. Sessions keep the terminal / changes /
/// preview pane; the inbox is the decision queue; every other destination
/// shows its selected panel full-width (the panels themselves are migrated
/// one by one in F3+, so they keep the workbench layout classes for now).
export function CenterV2(props: Props) {
  const destination = useShellStore((s) => s.destination);
  const tab = useWorkbenchStore((s) => s.tab);

  if (destination === "inbox") return <InboxView />;
  if (destination === "sessions") return <SessionsCenter {...props} />;
  return (
    <section
      className="flex h-full min-h-0 flex-col"
      aria-label={TAB_LABELS[tab]}
      data-testid="center-panel"
    >
      <div className="flex h-8 shrink-0 items-center border-b border-border px-3 font-sans text-small font-semibold text-foreground">
        {TAB_LABELS[tab]}
      </div>
      <div className="workbench-layout min-h-0 flex-1">
        <div className="workbench-content">
          <WorkbenchPanel tab={tab} />
        </div>
      </div>
    </section>
  );
}
