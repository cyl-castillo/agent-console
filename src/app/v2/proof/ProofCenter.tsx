import { useWorkbenchStore } from "../../../stores/workbenchStore";
import { WorkbenchPanel } from "../../panels";
import { TAB_LABELS } from "../destinations";
import { LedgerMinimap } from "./LedgerMinimap";

/// Proof destination: the map on top, the Turns or Ledger view below.
export function ProofCenter() {
  const tab = useWorkbenchStore((s) => s.tab);
  return (
    <section
      className="flex h-full min-h-0 flex-col"
      aria-label={TAB_LABELS[tab]}
      data-testid="center-panel"
    >
      <div className="flex h-8 shrink-0 items-center border-b border-border px-3 font-sans text-small font-semibold text-foreground">
        {TAB_LABELS[tab]}
      </div>
      <LedgerMinimap />
      <div className="workbench-layout min-h-0 flex-1">
        <div className="workbench-content">
          <WorkbenchPanel tab={tab} />
        </div>
      </div>
    </section>
  );
}
