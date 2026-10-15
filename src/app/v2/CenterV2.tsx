import type { Project } from "../../types/domain";
import type { TerminalSession } from "../../stores/terminalsStore";
import { cn } from "@/lib/cn";
import { useShellStore } from "../../stores/shellStore";
import { useWorkbenchStore } from "../../stores/workbenchStore";
import { SessionsCenter } from "./SessionsCenter";
import { ProofCenter } from "./proof/ProofCenter";
import { WorkbenchPanel } from "../panels";
import { InboxView } from "./InboxView";
import { TAB_LABELS } from "./destinations";

interface Props {
  project: Project;
  liveTerminals: TerminalSession[];
  activeTerminal: TerminalSession | undefined;
  activeStopped: boolean;
}

/// The centre column of the v2 shell. The Sessions zone is mounted for the
/// project's whole life and only hidden while another destination is up —
/// its PTYs (xterm instances, scrollback, selection) must survive a trip to
/// the Inbox, exactly as the v1 tab panes did. The other zones are cheap to
/// rebuild from their stores and mount on demand.
export function CenterV2(props: Props) {
  const destination = useShellStore((s) => s.destination);
  const tab = useWorkbenchStore((s) => s.tab);
  const sessions = destination === "sessions";

  return (
    <>
      <div
        className={cn("h-full min-h-0 min-w-0", !sessions && "hidden")}
        data-testid="zone-sessions"
        aria-hidden={!sessions || undefined}
      >
        <SessionsCenter {...props} shown={sessions} />
      </div>
      {destination === "inbox" && <InboxView />}
      {destination === "proof" && <ProofCenter />}
      {(destination === "work" || destination === "settings") && (
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
      )}
    </>
  );
}
