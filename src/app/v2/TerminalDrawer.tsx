import { Terminal } from "../../components/Terminal";
import { WorktreeBar } from "../../components/WorktreeBar";
import { cn } from "@/lib/cn";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { Button } from "../../ui/Button";

/// The PTYs, mounted once and for the session's whole life (a closed drawer
/// only hides them). The conversation is the stage; this is the engine room.
export function TerminalDrawer({
  liveTerminals,
  activeTerminal,
  activeStopped,
}: {
  liveTerminals: TerminalSession[];
  activeTerminal: TerminalSession | undefined;
  activeStopped: boolean;
}) {
  const open = useShellStore((s) => s.drawerOpen);
  const activeId = useTerminalsStore((s) => s.activeId);
  const resume = useTerminalsStore((s) => s.resume);
  return (
    <section
      aria-label="Terminal"
      data-testid="terminal-drawer"
      data-open={open || undefined}
      className={cn(
        "flex shrink-0 flex-col overflow-hidden border-t border-border bg-background",
        open ? "h-[40%] min-h-40" : "h-0 border-t-0",
      )}
    >
      <WorktreeBar />
      {activeStopped && activeTerminal && (
        <div className="flex items-center gap-3 px-3 py-2 font-sans text-small text-muted-foreground">
          <span>
            “{activeTerminal.name}” exited. Resume relaunches its shell and agent with the saved
            scrollback.
          </span>
          <Button size="sm" variant="primary" onClick={() => resume(activeTerminal.id)}>
            Resume session
          </Button>
        </div>
      )}
      <div className="terminals-stack" style={{ display: activeStopped ? "none" : undefined }}>
        {liveTerminals.map((s) => (
          <Terminal key={s.id} session={s} visible={open && s.id === activeId} />
        ))}
      </div>
    </section>
  );
}
