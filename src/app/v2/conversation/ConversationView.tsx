import { useEffect, useRef, useState } from "react";
import { MessageSquare, Square, TerminalSquare } from "lucide-react";
import { typeIntoActiveSession } from "../../../lib/termInput";
import type { TermInputDetail } from "../../../components/Terminal";
import { useChangesStore } from "../../../stores/changesStore";
import { useShellStore } from "../../../stores/shellStore";
import { useToastStore } from "../../../stores/toastStore";
import { useUIStore } from "../../../stores/uiStore";
import { useWorkbenchStore } from "../../../stores/workbenchStore";
import { Button } from "../../../ui/Button";
import { EmptyState } from "../../../ui/EmptyState";
import { Kbd } from "../../../ui/Kbd";
import { Textarea } from "../../../ui/Field";
import { Tooltip } from "../../../ui/Tooltip";
import { IconButton } from "../../../ui/IconButton";
import { newSession } from "../../actions";
import { TurnBlock } from "./blocks";
import { useSessionTimeline } from "./useSessionTimeline";

/// The conversation the app renders itself (F4): the session's turns from the
/// ledger, newest at the bottom and kept in view, a composer that types into
/// the PTY, and the terminal one keystroke away in the drawer.
export function ConversationView() {
  const { session, turns } = useSessionTimeline();
  const setCenterTab = useUIStore((s) => s.setTab);
  const selectFile = useChangesStore((s) => s.setSelected);
  const setWorkbenchTab = useWorkbenchStore((s) => s.setTab);
  const drawerOpen = useShellStore((s) => s.drawerOpen);
  const toggleDrawer = useShellStore((s) => s.toggleDrawer);
  const showToast = useToastStore((s) => s.show);
  const [draft, setDraft] = useState("");
  const [now, setNow] = useState(() => Date.now());
  const listRef = useRef<HTMLDivElement>(null);
  const liveTurn = turns.length > 0 && turns[turns.length - 1].endTs === null;
  const hasTurns = turns.length > 0;

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  // End-anchored: the first paint lands at the bottom (the newest turn), and
  // the view stays there as turns arrive unless the user scrolled up to read
  // (then nothing moves under them).
  const shown = useRef(0);
  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const first = shown.current === 0 && turns.length > 0;
    shown.current = turns.length;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (first || nearBottom) el.scrollTop = el.scrollHeight;
  }, [turns]);
  // The Sessions zone stays mounted while hidden (display:none → 0 px tall),
  // and the drawer changes the list's height: re-anchor to the end when the
  // box becomes visible for the first time or resizes while at the bottom.
  useEffect(() => {
    const el = listRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    let anchoredVisible = el.clientHeight > 0;
    let atBottom = true;
    const onScroll = () => {
      if (el.clientHeight > 0) atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    };
    const ro = new ResizeObserver(() => {
      if (el.clientHeight === 0) return;
      if (!anchoredVisible || atBottom) el.scrollTop = el.scrollHeight;
      anchoredVisible = true;
    });
    el.addEventListener("scroll", onScroll);
    ro.observe(el);
    return () => {
      el.removeEventListener("scroll", onScroll);
      ro.disconnect();
    };
    // Re-attach once turns exist: the first anchoring needs real content.
  }, [hasTurns]);

  const send = async () => {
    const text = draft.trim();
    if (!text) return;
    const ok = await typeIntoActiveSession(text, { submit: true });
    if (ok) setDraft("");
    else showToast("No live session to send to — copied to the clipboard instead", "info");
  };
  const interrupt = () => {
    if (!session) return;
    const detail: TermInputDetail = { sessionId: session.id, data: "\x1b" };
    window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
  };
  const openFile = (path: string) => {
    void selectFile(path);
    setCenterTab("changes");
  };
  const rerun = (prompt: string) => {
    setDraft(prompt);
  };
  const copy = (text: string) => {
    void navigator.clipboard?.writeText(text).then(
      () => showToast("Summary copied", "success"),
      () => showToast("Couldn't copy", "error"),
    );
  };

  if (!session) {
    return (
      <EmptyState
        className="h-full"
        icon={<MessageSquare />}
        title="No session selected"
        description="Pick a session on the left, or start one; its turns, tool calls, diffs and proof land here."
        action={
          <Button variant="primary" onClick={newSession}>
            New session
          </Button>
        }
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="conversation">
      <div ref={listRef} className="min-h-0 flex-1 overflow-auto">
        {turns.length === 0 ? (
          <EmptyState
            className="h-full"
            icon={<MessageSquare />}
            title="No turns yet"
            description="Send a prompt below. Each turn lands here as it happens: prompt, tools, files, tests, proof. If prompts go by and nothing shows up, check the hooks pill in the status bar."
          />
        ) : (
          <div className="divide-y divide-border">
            {turns.map((t, i) => (
              <TurnBlock
                key={t.turnId ?? i}
                turn={t}
                live={i === turns.length - 1 && t.endTs === null}
                now={now}
                onOpenFile={openFile}
                onCopy={() => copy(t.summary)}
                onRerun={() => rerun(t.prompt)}
                onLedger={() => setWorkbenchTab("proof")}
              />
            ))}
          </div>
        )}
      </div>
      <form
        className="flex shrink-0 flex-col gap-2 border-t border-border bg-surface-1 p-3"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
        data-testid="composer"
      >
        <Textarea
          aria-label="Prompt for the agent"
          placeholder={
            liveTurn
              ? "The agent is working — queue the next prompt, or interrupt"
              : "Tell the agent what to do next…"
          }
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          rows={2}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              void send();
            }
          }}
        />
        <div className="flex items-center gap-2">
          <Tooltip
            content={drawerOpen ? "Hide the terminal (Ctrl+`)" : "Show the terminal (Ctrl+`)"}
          >
            <IconButton
              label="Toggle terminal"
              icon={<TerminalSquare size={16} />}
              active={drawerOpen}
              onClick={toggleDrawer}
              data-testid="toggle-drawer"
            />
          </Tooltip>
          {liveTurn && (
            <Button size="sm" variant="danger" icon={<Square size={12} />} onClick={interrupt}>
              Interrupt
            </Button>
          )}
          <span className="flex-1" />
          <Button
            type="submit"
            variant="primary"
            size="sm"
            disabled={!draft.trim()}
            trailing={
              <>
                <Kbd>Ctrl</Kbd>
                <Kbd>⏎</Kbd>
              </>
            }
          >
            Send
          </Button>
        </div>
      </form>
    </div>
  );
}
