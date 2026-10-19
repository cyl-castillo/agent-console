import { useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import { Maximize2, Minimize2, PanelBottomClose, PanelBottomOpen } from "lucide-react";
import { Terminal } from "../../components/Terminal";
import { WorktreeBar } from "../../components/WorktreeBar";
import { cn } from "@/lib/cn";
import { DRAWER_LIMITS, useShellStore } from "../../stores/shellStore";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { Button } from "../../ui/Button";
import { Kbd } from "../../ui/Kbd";
import { Tooltip } from "../../ui/Tooltip";

/// The PTYs, mounted once and for the session's whole life (a closed drawer
/// only hides them). Open by default and the larger part of the centre: the
/// CLI is where trust, login, approvals and the agent's words happen; the
/// rendered conversation above is the companion, and can be folded away so
/// the terminal takes everything. Drag the strip to resize; Ctrl+` hides.
export function TerminalDrawer({
  liveTerminals,
  activeTerminal,
  activeStopped,
  shown = true,
  containerRef,
}: {
  liveTerminals: TerminalSession[];
  activeTerminal: TerminalSession | undefined;
  activeStopped: boolean;
  /// False while the Sessions zone itself is hidden (another destination is
  /// up): the PTYs stay mounted, xterm just stops fitting an invisible box.
  shown?: boolean;
  /// The Sessions centre, to size the drawer against while dragging.
  containerRef?: RefObject<HTMLDivElement | null>;
}) {
  const open = useShellStore((s) => s.drawerOpen);
  const pct = useShellStore((s) => s.drawerPct);
  const focus = useShellStore((s) => s.focus);
  const toggleFocus = useShellStore((s) => s.toggleFocus);
  const chatCollapsed = useShellStore((s) => s.chatCollapsed || s.focus);
  const toggleChat = useShellStore((s) => s.toggleChat);
  const setDrawerPct = useShellStore((s) => s.setDrawerPct);
  const activeId = useTerminalsStore((s) => s.activeId);
  const resume = useTerminalsStore((s) => s.resume);
  const [dragging, setDragging] = useState(false);
  const pending = useRef<number | null>(null);
  const sectionRef = useRef<HTMLElement>(null);

  const startDrag = (e: ReactPointerEvent<HTMLElement>) => {
    if (chatCollapsed) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging(true);
    pending.current = null;
  };
  const moveDrag = (e: ReactPointerEvent<HTMLElement>) => {
    if (!dragging) return;
    const box = containerRef?.current?.getBoundingClientRect();
    if (!box || box.height === 0) return;
    const [min, max] = DRAWER_LIMITS;
    const p = Math.max(min, Math.min(max, ((box.bottom - e.clientY) / box.height) * 100));
    pending.current = p;
    if (sectionRef.current) sectionRef.current.style.height = `${p}%`;
  };
  const endDrag = (e: ReactPointerEvent<HTMLElement>) => {
    if (!dragging) return;
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    setDragging(false);
    if (sectionRef.current) sectionRef.current.style.removeProperty("height");
    if (pending.current !== null) setDrawerPct(pending.current);
    pending.current = null;
  };

  const full = chatCollapsed;
  return (
    <section
      ref={sectionRef}
      aria-label="Terminal"
      data-testid="terminal-drawer"
      data-open={open || undefined}
      data-full={full || undefined}
      className={cn(
        "flex shrink-0 flex-col overflow-hidden border-t border-border bg-background",
        !open && "h-0 border-t-0",
        open && full && "min-h-0 flex-1",
        open && !full && "min-h-40",
      )}
      style={open && !full ? { height: `${pct}%` } : undefined}
    >
      <div
        className={cn(
          "flex h-7 shrink-0 select-none items-center gap-2 border-b border-border bg-surface-1 px-2 font-sans text-caption text-muted-foreground",
          !full && "cursor-row-resize",
          dragging && "bg-[var(--hover)]",
        )}
        data-testid="drawer-strip"
        onPointerDown={startDrag}
        onPointerMove={moveDrag}
        onPointerUp={endDrag}
        onDoubleClick={() => !full && setDrawerPct(null)}
        title={full ? undefined : "Drag to resize · double-click to reset"}
      >
        {/* The grip is the separator; the buttons are its neighbours, not
            its children (a separator may not contain focusable content). */}
        <div
          className="flex min-w-0 flex-1 items-center gap-2 self-stretch"
          role={full ? undefined : "separator"}
          aria-orientation={full ? undefined : "horizontal"}
          aria-label={full ? undefined : "Resize the terminal"}
        >
          <span className="font-medium text-foreground">Terminal</span>
          {activeTerminal && <span className="truncate">{activeTerminal.name}</span>}
        </div>
        <Tooltip
          content={
            chatCollapsed
              ? "Show the conversation above the terminal"
              : "Hide the conversation: the terminal takes the whole centre"
          }
        >
          <Button
            size="sm"
            variant="ghost"
            disabled={focus}
            icon={chatCollapsed ? <PanelBottomClose size={14} /> : <PanelBottomOpen size={14} />}
            onClick={toggleChat}
            onPointerDown={(e) => e.stopPropagation()}
            data-testid="toggle-chat"
          >
            {chatCollapsed ? "Show chat" : "Hide chat"}
          </Button>
        </Tooltip>
        <Tooltip
          content={
            focus
              ? "Leave focus mode: bring the rail, lists and chat back"
              : "Focus mode: the terminal alone — rail, lists, chat and top bar out of the way (Ctrl+Shift+F)"
          }
        >
          <Button
            size="sm"
            variant="ghost"
            icon={focus ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
            onClick={toggleFocus}
            onPointerDown={(e) => e.stopPropagation()}
            data-testid="toggle-focus"
          >
            {focus ? "Leave focus" : "Focus"}
          </Button>
        </Tooltip>
        <Tooltip content="Hide or show the terminal">
          <span className="inline-flex items-center gap-0.5">
            <Kbd>Ctrl</Kbd>
            <Kbd>`</Kbd>
          </span>
        </Tooltip>
      </div>
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
          <Terminal key={s.id} session={s} visible={shown && open && s.id === activeId} />
        ))}
      </div>
    </section>
  );
}
