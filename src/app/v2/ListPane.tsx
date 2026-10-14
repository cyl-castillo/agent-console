import { LeftSidebar } from "../../components/LeftSidebar";
import { isTabEnabled } from "../../stores/modulesStore";
import { useRoundtableStore } from "../../stores/roundtableStore";
import { useShellStore } from "../../stores/shellStore";
import { useWorkbenchStore } from "../../stores/workbenchStore";
import { Row } from "../../ui/Row";
import { SectionHeader } from "../../ui/SectionHeader";
import { destinationDef, TAB_LABELS } from "./destinations";

/// The second column: what the destination offers. Sessions reuse the
/// existing workspace sidebar (sessions, rooms, changes, files); the other
/// destinations list their panels as dense rows.
export function ListPane() {
  const destination = useShellStore((s) => s.destination);
  const tab = useWorkbenchStore((s) => s.tab);
  const setTab = useWorkbenchStore((s) => s.setTab);
  const openRoom = useRoundtableStore((s) => s.openRoom);

  if (destination === "sessions") {
    return (
      <div className="flex h-full min-h-0 flex-col [&>.panel.left]:h-full [&>.panel.left]:w-full">
        <LeftSidebar
          onOpenRoom={(id) => {
            void openRoom(id);
            setTab("roundtable");
          }}
        />
      </div>
    );
  }
  const def = destinationDef(destination);
  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto py-1" data-testid="list-pane">
      {def.sections.map((sec) => {
        const tabs = sec.tabs.filter(isTabEnabled);
        if (tabs.length === 0) return null;
        return (
          <section key={sec.label} className="mb-2">
            <SectionHeader title={sec.label} />
            {tabs.map((t) => (
              <Row
                key={t}
                title={TAB_LABELS[t]}
                selected={t === tab}
                onClick={() => setTab(t)}
                data-testid={`list-${t}`}
              />
            ))}
          </section>
        );
      })}
    </div>
  );
}
