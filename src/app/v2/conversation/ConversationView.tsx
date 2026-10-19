import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ArrowDown, Clock, MessageSquare, Square, TerminalSquare, X } from "lucide-react";
import { typeIntoActiveSession } from "../../../lib/termInput";
import type { TermInputDetail } from "../../../components/Terminal";
import { useChangesStore } from "../../../stores/changesStore";
import { ipc } from "../../../ipc/tauri";
import { useSessionStore } from "../../../stores/sessionStore";
import { useTerminalsStore } from "../../../stores/terminalsStore";
import { useShellStore } from "../../../stores/shellStore";
import { useToastStore } from "../../../stores/toastStore";
import { useUIStore } from "../../../stores/uiStore";
import { useWorkbenchStore } from "../../../stores/workbenchStore";
import { queueFor, usePromptQueueStore } from "../../../stores/promptQueueStore";
import { Button } from "../../../ui/Button";
import { EmptyState } from "../../../ui/EmptyState";
import { Kbd } from "../../../ui/Kbd";
import { Textarea } from "../../../ui/Field";
import { Tooltip } from "../../../ui/Tooltip";
import { IconButton } from "../../../ui/IconButton";
import { newSession } from "../../actions";
import { TurnBlock } from "./blocks";
import { StatusGlyph } from "../../../ui/StatusGlyph";
import { useSessionTimeline } from "./useSessionTimeline";
import { useTranscriptWatch } from "./useTranscriptWatch";
import { saidText, streamFor } from "./stream";
import { SessionCards } from "./SessionCards";
import { useApprovalStore } from "../../../stores/approvalStore";
import { useHooksHealthStore } from "../../../stores/hooksHealthStore";
import { isBooting, signalsFor, useTermSignalsStore } from "../../../stores/termSignalsStore";
import {
  UNWITNESSED_GRACE_MS,
  holdsQueue,
  unwitnessed,
  type PendingPrompt,
} from "./pendingPrompts";

/// Claude has no interrupt hook: after Esc the ledger's turn stays open until
/// the next prompt closes it. Interrupt is taken at its word after this long
/// with no close — the turn shows as cut short and the queue moves on.
export const INTERRUPT_GRACE_MS = 3000;
/// Closer than this to the end counts as "reading the latest".
const NEAR_BOTTOM_PX = 80;

/// The conversation the app renders itself (F4): the session's turns from the
/// ledger, newest at the bottom and kept in view, a composer that types into
/// the PTY, and the terminal one keystroke away in the drawer.
export function ConversationView() {
  const { session, turns } = useSessionTimeline();
  // F5: the agent's words as the CLI writes them, per turn.
  const transcript = useTranscriptWatch(session);
  const setCenterTab = useUIStore((s) => s.setTab);
  const selectFile = useChangesStore((s) => s.setSelected);
  const setWorkbenchTab = useWorkbenchStore((s) => s.setTab);
  const drawerOpen = useShellStore((s) => s.drawerOpen);
  const toggleDrawer = useShellStore((s) => s.toggleDrawer);
  const showToast = useToastStore((s) => s.show);
  const [draft, setDraft] = useState("");
  const [now, setNow] = useState(() => Date.now());
  const listRef = useRef<HTMLDivElement>(null);
  const draftRef = useRef<HTMLTextAreaElement>(null);
  const goTo = useShellStore((s) => s.goTo);
  // Ctrl+E / palette "compose prompt": the conversation's draft IS the
  // composer — bring Sessions up and focus it.
  useEffect(
    () =>
      useUIStore.subscribe((s, prev) => {
        if (!s.composerOpen || prev.composerOpen) return;
        goTo("sessions");
        useUIStore.getState().setComposerOpen(false);
        window.setTimeout(() => draftRef.current?.focus(), 0);
      }),
    [goTo],
  );
  // What was sent and not yet witnessed as a prompt: shown at once, so the
  // click never looks like nothing happened (the ledger line arrives through
  // the hook a moment later, or never if the hooks are off — then the echo
  // stays with its note).
  const [pending, setPending] = useState<PendingPrompt[]>([]);
  useEffect(() => {
    if (pending.length === 0) return;
    const left = unwitnessed(pending, turns);
    if (left.length !== pending.length) setPending(left);
  }, [turns, pending]);

  const last = turns.length > 0 ? turns[turns.length - 1] : null;
  const lastOpen = last !== null && last.endTs === null;
  const hasTurns = turns.length > 0;

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);

  // Interrupt, taken at its word: Codex closes the turn through its hook,
  // Claude never does. Pressing it marks the open turn "stopping"; after the
  // grace with no close the turn shows as cut short and liveTurn lets go.
  const [stopping, setStopping] = useState<{ turnId: string | null; at: number } | null>(null);
  useEffect(() => {
    if (stopping && (!lastOpen || last?.turnId !== stopping.turnId)) setStopping(null);
  }, [stopping, lastOpen, last?.turnId]);
  const stoppedLocally = stopping !== null && lastOpen && now - stopping.at >= INTERRUPT_GRACE_MS;
  const liveTurn = lastOpen && !stoppedLocally;

  // End-anchored. Whether the reader is at the end is measured when THEY
  // scroll (before anything grows under them), never after the DOM grew —
  // measured after, a long reply had already pushed the end out of reach and
  // the view stayed put without a word. Away from the end, growth raises a
  // pill instead of moving the view.
  const atBottom = useRef(true);
  const [unseen, setUnseen] = useState(false);
  const measure = () => {
    const el = listRef.current;
    if (!el || el.clientHeight === 0) return;
    atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
    if (atBottom.current) setUnseen(false);
  };
  const toBottom = () => {
    const el = listRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    atBottom.current = true;
    setUnseen(false);
  };
  const shown = useRef({ turns: 0, height: 0 });
  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const first = shown.current.turns === 0 && turns.length > 0;
    const grew = el.scrollHeight > shown.current.height;
    shown.current = { turns: turns.length, height: el.scrollHeight };
    if (first || atBottom.current) toBottom();
    else if (grew && el.clientHeight > 0) setUnseen(true);
  }, [turns, pending]);
  // The Sessions zone stays mounted while hidden (display:none → 0 px tall),
  // and the drawer changes the list's height: re-anchor to the end when the
  // box becomes visible for the first time or resizes while at the bottom.
  useEffect(() => {
    const el = listRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    let anchoredVisible = el.clientHeight > 0;
    const ro = new ResizeObserver(() => {
      if (el.clientHeight === 0) return;
      if (!anchoredVisible || atBottom.current) el.scrollTop = el.scrollHeight;
      anchoredVisible = true;
    });
    ro.observe(el);
    return () => ro.disconnect();
    // Re-attach once turns exist: the first anchoring needs real content.
  }, [hasTurns]);

  // D10: a different identity reads what the turn changed and argues
  // against it; the verdict lands in the ledger and the Inbox.
  const review = (termId: string, turnId: string) => {
    const root = useSessionStore.getState().project?.root;
    if (!root) return;
    showToast("Review requested — the verdict will land in the Inbox", "info");
    ipc
      .reviewTurn(root, termId, turnId)
      .catch((e) => showToast(`Review failed: ${String(e)}`, "error"));
  };

  const deliver = async (text: string): Promise<boolean> => {
    const ok = await typeIntoActiveSession(text, { submit: true });
    if (ok) setPending((p) => [...p, { text, ts: Date.now() }]);
    return ok;
  };
  const resume = useTerminalsStore((s) => s.resume);
  const send = async () => {
    const text = draft.trim();
    if (!text) return;
    if (session && !sessionLive) {
      // F4: a stopped session resumes from the composer; the prompt waits
      // for the CLI and goes out as its own turn.
      resume(session.id);
      enqueue(session.id, text);
      setDraft("");
      showToast("Resuming the session — your prompt goes out when the CLI is up", "info");
      draftRef.current?.focus();
      return;
    }
    if (booting && session) {
      enqueue(session.id, text);
      setDraft("");
      showToast("Queued — the CLI is still starting; it goes out at the prompt", "info");
      return;
    }
    // Sending is reading the latest: the echo lands in view.
    atBottom.current = true;
    const ok = await deliver(text);
    if (ok) {
      setDraft("");
      showToast(`Sent to ${session?.name ?? "the session"}`, "success");
      draftRef.current?.focus();
    } else showToast("No live session to send to — copied to the clipboard instead", "info");
  };

  // Ctrl+Enter: queue for later. The prompt waits here, visible and
  // removable, and goes out on its own when the session is live and has no
  // live turn — one at a time, so each lands as its own turn.
  const sessionLive = session?.status === "live";
  // F3: while the CLI boots, nothing is typed at the shell — sends queue.
  const sig = useTermSignalsStore((s) => signalsFor(s.bySession, session?.id));
  const booting = sessionLive && isBooting(sig, now);
  const waitingApproval = useApprovalStore((s) => s.queue.some((r) => r.termId === session?.id));
  const hooksVerdict = useHooksHealthStore((s) => s.verdict.kind);
  const queued = usePromptQueueStore((s) => queueFor(s.bySession, session?.id));
  const enqueue = usePromptQueueStore((s) => s.enqueue);
  const removeQueued = usePromptQueueStore((s) => s.remove);
  const queueLater = () => {
    const text = draft.trim();
    if (!text || !session) return;
    enqueue(session.id, text);
    setDraft("");
    showToast(
      !sessionLive
        ? "Queued — goes out when the session is live again"
        : liveTurn
          ? "Queued — goes out when this turn ends"
          : "Queued — going out now",
      "info",
    );
    draftRef.current?.focus();
  };
  // The last send holds the queue until it is witnessed, or for the grace
  // period when nothing witnesses it (hooks off).
  const queueHeld = holdsQueue(pending, now);
  const sending = useRef(false);
  useEffect(() => {
    if (!session || !sessionLive || booting || liveTurn || queueHeld || sending.current) return;
    const next = queued[0];
    if (!next) return;
    // The prompt leaves the queue only once it is in the PTY. A failed
    // delivery leaves it where it is — re-queueing it changed the queue,
    // which re-ran this effect, which failed again: a hot loop that froze
    // the webview whenever the session had stopped.
    sending.current = true;
    void deliver(next.text).then((ok) => {
      sending.current = false;
      if (!ok) return;
      usePromptQueueStore.getState().remove(session.id, next.id);
      showToast(`Queued prompt sent to ${session.name}`, "success");
    });
    // deliver/showToast are stable enough for this effect's purpose.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session?.id, sessionLive, booting, liveTurn, queued, queueHeld]);
  const interrupt = () => {
    if (!session) return;
    const detail: TermInputDetail = { sessionId: session.id, data: "\x1b" };
    window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
    setStopping({ turnId: last?.turnId ?? null, at: Date.now() });
  };
  const openTerminal = () => {
    if (!drawerOpen) toggleDrawer();
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
      <div className="relative min-h-0 flex-1">
        <div
          ref={listRef}
          onScroll={measure}
          className="h-full overflow-auto"
          role="log"
          aria-label="Conversation"
          data-testid="conversation-log"
        >
          {turns.length === 0 ? (
            <EmptyState
              className="h-full"
              icon={<MessageSquare />}
              title="No turns yet"
              description={
                hooksVerdict === "off"
                  ? "The hooks are not installed, so nothing the agent does can land here. Install them, or work in the terminal."
                  : "Send a prompt below. Each turn lands here as it happens: prompt, tools, files, tests, proof."
              }
              action={
                <div className="flex items-center gap-2">
                  {hooksVerdict === "off" && (
                    <Button
                      variant="primary"
                      onClick={() =>
                        void ipc
                          .hooksInstall()
                          .then(() => showToast("Hooks installed", "success"))
                          .catch((e) => showToast(`Couldn't install hooks: ${String(e)}`, "error"))
                      }
                    >
                      Install hooks
                    </Button>
                  )}
                  <Button
                    variant="ghost"
                    icon={<TerminalSquare size={14} />}
                    onClick={openTerminal}
                  >
                    Open terminal
                  </Button>
                </div>
              }
            />
          ) : (
            <div className="divide-y divide-border">
              {turns.map((t, i) => {
                const isLast = i === turns.length - 1;
                // Interrupt taken at its word: the open turn reads as cut short
                // once the grace passed with no close from the ledger.
                const turn =
                  isLast && stoppedLocally && stopping
                    ? { ...t, endTs: stopping.at, interrupted: true }
                    : t;
                const stream = streamFor(transcript, turn);
                return (
                  <TurnBlock
                    key={t.turnId ?? i}
                    turn={turn}
                    live={isLast && liveTurn}
                    waitingApproval={isLast && liveTurn && waitingApproval}
                    now={now}
                    stream={stream}
                    onCopyText={copy}
                    onOpenFile={openFile}
                    onCopy={() => copy(saidText(stream, t.summary))}
                    onRerun={() => rerun(t.prompt)}
                    onLedger={() => setWorkbenchTab("proof")}
                    onReview={t.termId && t.turnId ? () => review(t.termId!, t.turnId!) : undefined}
                    onMore={openTerminal}
                  />
                );
              })}
            </div>
          )}
          <SessionCards session={session} now={now} lastTurnTs={last?.ts ?? null} />
          {pending.map((p) => (
            <div
              key={p.ts}
              className="flex flex-col gap-1 border-t border-border px-3 py-2 font-sans text-body"
              data-testid="pending-prompt"
              aria-live="polite"
            >
              <div className="flex items-center gap-2 text-caption text-muted-foreground">
                <StatusGlyph state="working" />
                <span>you · sent to the terminal</span>
                {now - p.ts > UNWITNESSED_GRACE_MS && (
                  <>
                    <span className="text-warn">· not witnessed yet</span>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={openTerminal}
                      data-testid="echo-open-terminal"
                    >
                      Open terminal
                    </Button>
                  </>
                )}
              </div>
              <div className="whitespace-pre-wrap text-foreground">{p.text}</div>
            </div>
          ))}
        </div>
        {unseen && (
          <button
            type="button"
            onClick={toBottom}
            className="unbutton absolute bottom-2 left-1/2 inline-flex -translate-x-1/2 items-center gap-1 rounded-pill border border-border bg-surface-2 px-3 py-1 font-sans text-small text-foreground shadow-md hover:bg-[var(--hover)] focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring"
            data-testid="new-reply-pill"
          >
            <ArrowDown size={14} aria-hidden />
            New reply
          </button>
        )}
      </div>
      {!sessionLive && (
        <div
          className="flex shrink-0 items-center gap-3 border-t border-border bg-surface-1 px-3 py-2 font-sans text-small text-muted-foreground"
          data-testid="resume-banner"
          role="status"
        >
          <span>
            This session is stopped. Resume relaunches its shell and agent with the saved
            scrollback; sending a prompt resumes it too.
          </span>
          <Button size="sm" variant="primary" onClick={() => resume(session.id)}>
            Resume session
          </Button>
        </div>
      )}
      <form
        className="flex shrink-0 flex-col gap-2 border-t border-border bg-surface-1 p-3"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
        data-testid="composer"
      >
        <Textarea
          ref={draftRef}
          aria-label="Prompt for the agent"
          placeholder={
            !sessionLive
              ? "Stopped — Enter resumes the session and sends"
              : liveTurn
                ? "The agent is working — queue the next prompt, or interrupt"
                : "Tell the agent what to do next…"
          }
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          rows={2}
          onKeyDown={(e) => {
            if (e.key !== "Enter" || e.nativeEvent.isComposing) return;
            // Enter sends, like the terminal it stands in for. Ctrl/Cmd+Enter
            // queues for later (sent when the turn ends). Shift/Alt+Enter
            // adds a line.
            if (e.ctrlKey || e.metaKey) {
              e.preventDefault();
              queueLater();
            } else if (!e.shiftKey && !e.altKey) {
              e.preventDefault();
              void send();
            }
          }}
        />
        {queued.length > 0 && (
          <ul className="m-0 flex list-none flex-col gap-1 p-0" data-testid="prompt-queue">
            {queued.map((q, i) => (
              <li
                key={q.id}
                className="flex items-start gap-2 rounded-2 border border-dashed border-border px-2 py-1 font-sans text-small text-muted-foreground"
              >
                <Clock size={14} className="mt-0.5 shrink-0" aria-hidden />
                <span className="min-w-0 flex-1 whitespace-pre-wrap text-foreground">{q.text}</span>
                <span className="shrink-0 text-caption text-faint-foreground">
                  {i !== 0
                    ? `#${i + 1}`
                    : !sessionLive
                      ? "waiting for the session to resume"
                      : liveTurn
                        ? "next, when this turn ends"
                        : queueHeld
                          ? "next, after the last send"
                          : "sending…"}
                </span>
                <IconButton
                  size="sm"
                  label="Remove from queue"
                  icon={<X size={14} />}
                  onClick={() => session && removeQueued(session.id, q.id)}
                />
              </li>
            ))}
          </ul>
        )}
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
            <Button
              size="sm"
              variant="danger"
              icon={<Square size={12} />}
              onClick={interrupt}
              disabled={stopping !== null}
              data-testid="interrupt"
            >
              {stopping ? "Stopping…" : "Interrupt"}
            </Button>
          )}
          <span className="flex-1" />
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={!draft.trim() || !session}
            trailing={
              <>
                <Kbd>Ctrl</Kbd>
                <Kbd>⏎</Kbd>
              </>
            }
            title="Queue for later: sent on its own when the current turn ends"
            onClick={queueLater}
          >
            Queue
          </Button>
          <Button
            type="submit"
            variant="primary"
            size="sm"
            disabled={!draft.trim()}
            trailing={<Kbd>⏎</Kbd>}
            title="Enter sends · Ctrl+Enter queues for later · Shift+Enter adds a line"
          >
            Send
          </Button>
        </div>
      </form>
    </div>
  );
}
