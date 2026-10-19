import { useRef } from "react";
import { ChangesView } from "../../components/ChangesView";
import { Preview } from "../../components/Preview";
import { FileTree } from "../../components/FileTree";
import { useSessionStore } from "../../stores/sessionStore";
import type { Project } from "../../types/domain";
import type { TerminalSession } from "../../stores/terminalsStore";
import { useChangesStore } from "../../stores/changesStore";
import { useShellStore } from "../../stores/shellStore";
import { cn } from "@/lib/cn";
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
/// the terminal as a drawer under all three — open by default and the
/// larger part, foldable to the whole centre. The "terminal" tab id is kept
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
  const tree = useSessionStore((s) => s.tree);
  const chatCollapsed = useShellStore((s) => s.chatCollapsed || s.focus);
  const containerRef = useRef<HTMLDivElement>(null);
  return (
    <div ref={containerRef} className="flex h-full min-h-0 flex-col" data-testid="sessions-center">
      <Tabs<CenterTab>
        label="Session views"
        value={tab}
        onValueChange={setTab}
        className={cn("min-h-0 flex-1", chatCollapsed && "hidden")}
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
              {tree && (
                <aside
                  className="w-56 shrink-0 overflow-auto border-r border-border px-1 py-1"
                  aria-label="Project files"
                  data-testid="preview-files"
                >
                  <FileTree root={tree} />
                </aside>
              )}
              <div className="min-w-0 flex-1">
                <Preview />
              </div>
            </div>
          ),
        }}
      </Tabs>
      <TerminalDrawer
        liveTerminals={liveTerminals}
        activeTerminal={activeTerminal}
        activeStopped={activeStopped}
        shown={shown}
        containerRef={containerRef}
      />
    </div>
  );
}
