import { ChangesView } from "../../components/ChangesView";
import { Preview } from "../../components/Preview";
import type { Project } from "../../types/domain";
import type { TerminalSession } from "../../stores/terminalsStore";
import { useChangesStore } from "../../stores/changesStore";
import { useUIStore, type CenterTab } from "../../stores/uiStore";
import { Tabs } from "../../ui/Tabs";
import { ConversationView } from "./conversation/ConversationView";
import { TerminalDrawer } from "./TerminalDrawer";

interface Props {
  project: Project;
  liveTerminals: TerminalSession[];
  activeTerminal: TerminalSession | undefined;
  activeStopped: boolean;
  /// False while another destination is up (the zone stays mounted, hidden).
  shown?: boolean;
}

/// Sessions in the v2 shell: Conversation / Changes / Preview as real tabs,
/// the terminal as a drawer under all three. The "terminal" tab id is kept
/// (palette, shortcuts, voice all use it) and means the conversation here.
export function SessionsCenter({
  liveTerminals,
  activeTerminal,
  activeStopped,
  shown = true,
}: Props) {
  const tab = useUIStore((s) => s.tab);
  const setTab = useUIStore((s) => s.setTab);
  const changes = useChangesStore((s) => s.status?.changes.length ?? 0);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <Tabs<CenterTab>
        label="Session views"
        value={tab}
        onValueChange={setTab}
        className="min-h-0 flex-1"
        items={[
          { value: "terminal", label: "Conversation" },
          { value: "changes", label: "Changes", count: changes },
          { value: "preview", label: "Preview" },
        ]}
      >
        {{
          terminal: <ConversationView />,
          changes: (
            <div className="tab-pane flex h-full">
              <ChangesView />
            </div>
          ),
          preview: (
            <div className="tab-pane flex h-full">
              <Preview />
            </div>
          ),
        }}
      </Tabs>
      <TerminalDrawer
        liveTerminals={liveTerminals}
        activeTerminal={activeTerminal}
        activeStopped={activeStopped}
        shown={shown}
      />
    </div>
  );
}
