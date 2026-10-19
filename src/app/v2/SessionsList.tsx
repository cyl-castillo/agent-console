import { useMemo, useState } from "react";
import { Archive, ChevronRight, FileText, FolderTree, Plus, RotateCcw, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { AgentModelChooser } from "../../components/SessionList";
import { FileTree } from "../../components/FileTree";
import { createSession } from "../../lib/createSession";
import { confirmDialog } from "../../stores/confirmStore";
import { useLiveStatusStore } from "../../stores/liveStatusStore";
import { useHandoffStore } from "../../stores/handoffStore";
import { useModelStore } from "../../stores/modelStore";
import { DEFAULT_AGENT } from "../../agents/profiles";
import { useProofStore } from "../../stores/proofStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { Badge } from "../../ui/Chip";
import { IconButton } from "../../ui/IconButton";
import { SectionHeader } from "../../ui/SectionHeader";
import { StatusGlyph, type AgentState } from "../../ui/StatusGlyph";
import { Tooltip } from "../../ui/Tooltip";
import { useInbox } from "./useInbox";

/// The vocabulary of a session's state, in the order a glance should read
/// it: ● needs you · ✻ working · ∙ idle · ○ stopped. Each group header
/// carries its glyph, so the legend is the list itself.
const GROUPS: { state: AgentState; title: string }[] = [
  { state: "needs-you", title: "Needs you" },
  { state: "working", title: "Working" },
  { state: "idle", title: "Idle" },
  { state: "stopped", title: "Stopped" },
];
/// Live sessions beyond which attention measurably degrades (2–4 in focus).
const WIP_SOFT_LIMIT = 4;

function relative(ms: number, now: number): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  return h < 48 ? `${h}h` : `${Math.round(h / 24)}d`;
}
const absolute = (ms: number) =>
  new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/// One line that says what the session is about: the last turn's summary,
/// else its prompt, else where it runs.
function useSummaries(): Map<string, string> {
  const events = useProofStore((s) => s.events);
  return useMemo(() => {
    const out = new Map<string, string>();
    for (const e of events) {
      if (!e.termId) continue;
      const p = e.payload as Record<string, unknown>;
      if (e.kind === "turn_end" && typeof p.summary === "string" && p.summary)
        out.set(e.termId, p.summary);
      else if (e.kind === "prompt" && typeof p.prompt === "string" && p.prompt)
        out.set(e.termId, p.prompt);
    }
    return out;
  }, [events]);
}

function SessionRow({
  session,
  state,
  summary,
  cost,
  lastActive,
  now,
  active,
  onActivate,
  onArchive,
  onRestore,
  onClose,
  onHandoff,
}: {
  session: TerminalSession;
  state: AgentState;
  summary?: string;
  cost?: number;
  lastActive: number;
  now: number;
  active: boolean;
  onActivate: () => void;
  onArchive?: () => void;
  onRestore?: () => void;
  onClose: () => void;
  onHandoff?: () => void;
}) {
  return (
    <div
      className={cn(
        "group relative transition-colors dur-1 hover:bg-[var(--hover)]",
        active && "bg-[var(--accent-12)]",
      )}
      data-testid={`session-${session.id}`}
      data-state={state}
    >
      <button
        type="button"
        onClick={onActivate}
        aria-current={active ? "true" : undefined}
        className="unbutton flex min-h-9 w-full items-center gap-2 px-3 py-1 text-left font-sans text-body text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
      >
        <StatusGlyph state={state} />
        <span className="min-w-0 flex-1">
          <span className="block truncate" data-testid="session-name">
            {session.name}
          </span>
          {(session.worktree || summary) && (
            <span className="block truncate text-small text-muted-foreground">
              {session.worktree && (
                <span className="font-code text-caption text-faint-foreground">
                  {session.worktree.branch}
                </span>
              )}
              {session.worktree && summary && " · "}
              {summary}
            </span>
          )}
        </span>
        <span
          className="shrink-0 text-right font-code text-caption text-muted-foreground tabular-nums"
          title={`Last activity ${absolute(lastActive)}`}
        >
          {cost !== undefined && <span className="block">${cost.toFixed(2)}</span>}
          <span className="block">{relative(lastActive, now)}</span>
        </span>
      </button>
      {/* Out of the row's flow: the name keeps the whole width, and the
          actions appear over the age on hover or keyboard focus. */}
      <span
        className="absolute inset-y-0 right-1 flex items-center gap-0.5 rounded-1 bg-[var(--hover)] opacity-0 transition-opacity dur-1 focus-within:opacity-100 group-hover:opacity-100"
        data-testid="session-actions"
      >
        {onHandoff && (
          <Tooltip content="Ask the agent for a handoff note — saved as a memory when its turn ends">
            <IconButton
              size="sm"
              label={`Ask ${session.name} for a handoff note`}
              icon={<FileText size={14} />}
              onClick={onHandoff}
            />
          </Tooltip>
        )}
        {onArchive && (
          <Tooltip content="Archive — hide in History, resumable anytime">
            <IconButton
              size="sm"
              label={`Archive ${session.name}`}
              icon={<Archive size={14} />}
              onClick={onArchive}
            />
          </Tooltip>
        )}
        {onRestore && (
          <Tooltip content="Restore — back to the list, resumed">
            <IconButton
              size="sm"
              label={`Restore ${session.name}`}
              icon={<RotateCcw size={14} />}
              onClick={onRestore}
            />
          </Tooltip>
        )}
        <Tooltip content={session.status === "live" ? "Close — kills the process" : "Remove"}>
          <IconButton
            size="sm"
            label={`Close ${session.name}`}
            icon={<X size={14} />}
            onClick={onClose}
          />
        </Tooltip>
      </span>
    </div>
  );
}

/// The Sessions list of the v2 shell: grouped by attention state, one line of
/// meaning per row, cost and age, a visible WIP count, History for archived
/// sessions, and the project files underneath.
export function SessionsList() {
  const project = useSessionStore((s) => s.project);
  const tree = useSessionStore((s) => s.tree);
  const { byState, lastActive, now, activeId } = useInbox();
  const summaries = useSummaries();
  const byTerm = useLiveStatusStore((s) => s.byTerm);
  const sessions = useTerminalsStore((s) => s.sessions);
  const setActive = useTerminalsStore((s) => s.setActive);
  const archive = useTerminalsStore((s) => s.archive);
  const resume = useTerminalsStore((s) => s.resume);
  const close = useTerminalsStore((s) => s.close);
  const requestHandoff = useHandoffStore((s) => s.request);
  const defaultAgentFor = useModelStore((s) => s.defaultAgentFor);
  const defaultFor = useModelStore((s) => s.defaultFor);
  const choosing = useShellStore((s) => s.chooserOpen);
  const setChoosing = useShellStore((s) => s.setChooserOpen);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [filesOpen, setFilesOpen] = useState(false);

  const live = sessions.filter((s) => s.status === "live" && !s.archived).length;
  const archived = sessions.filter((s) => s.archived);
  const needsYou = byState.get("needs-you")?.length ?? 0;
  const overWip = live > WIP_SOFT_LIMIT;

  const closeSession = async (s: TerminalSession) => {
    if (
      s.status === "live" &&
      !(await confirmDialog({
        title: "Close session",
        message: `Close session "${s.name}"? Process will be killed.`,
        confirmLabel: "Close",
        danger: true,
      }))
    )
      return;
    await close(s.id);
  };

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="sessions-list">
      <div className="flex h-8 shrink-0 items-center gap-2 px-3 font-sans">
        <span className="text-caption font-semibold uppercase tracking-wide text-faint-foreground">
          Sessions
        </span>
        <Tooltip
          content={
            overWip
              ? `${live} live sessions — past ${WIP_SOFT_LIMIT}, attention splits`
              : `${live} live session${live === 1 ? "" : "s"}`
          }
        >
          <span>
            <Badge
              count={live}
              className={cn(overWip ? "bg-warn text-background" : "bg-surface-3 text-foreground")}
            />
          </span>
        </Tooltip>
        {needsYou > 0 && <span className="text-caption text-warn">{needsYou} need you</span>}
        <span className="ml-auto" />
        <Tooltip content="New session · Ctrl+T — Enter starts with the last agent and model">
          <IconButton
            size="sm"
            label="New session"
            icon={<Plus size={14} />}
            active={choosing}
            onClick={() => setChoosing(!choosing)}
            data-testid="sessions-new"
          />
        </Tooltip>
      </div>
      {choosing && project && (
        <div
          className="border-b border-border px-2 pb-2"
          data-testid="session-chooser"
          tabIndex={-1}
          ref={(el) => el?.focus()}
          onKeyDown={(e) => {
            // One path, one key: Enter starts with what you used last.
            if (e.key === "Enter" && e.target === e.currentTarget) {
              e.preventDefault();
              const agent = defaultAgentFor(project.root) ?? DEFAULT_AGENT;
              void createSession(agent, defaultFor(project.root, agent)).then(
                (ok) => ok && setChoosing(false),
              );
            }
            if (e.key === "Escape") setChoosing(false);
          }}
        >
          <AgentModelChooser
            projectRoot={project.root}
            lastAgent={defaultAgentFor(project.root) ?? DEFAULT_AGENT}
            onPick={(agent, model, wt, folder) => {
              void createSession(agent, model, wt, folder).then((ok) => ok && setChoosing(false));
            }}
            onCancel={() => setChoosing(false)}
          />
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        {GROUPS.map(({ state, title }) => {
          const list = byState.get(state) ?? [];
          if (list.length === 0) return null;
          return (
            <section key={state} data-testid={`sessions-${state}`}>
              <SectionHeader
                title={title}
                count={list.length}
                leading={<StatusGlyph state={state} />}
              />
              {list.map((s) => (
                <SessionRow
                  key={s.id}
                  session={s}
                  state={state}
                  summary={summaries.get(s.id)}
                  cost={byTerm[s.id]?.costUsd}
                  lastActive={lastActive.get(s.id) ?? s.createdAtMs}
                  now={now}
                  active={s.id === activeId}
                  onActivate={() => (s.status === "stopped" ? resume(s.id) : setActive(s.id))}
                  // A live session is closed, not archived: archiving hides
                  // a process still running.
                  onArchive={s.status === "live" ? undefined : () => archive(s.id)}
                  onClose={() => void closeSession(s)}
                  onHandoff={
                    s.status === "live" && s.agent ? () => requestHandoff(s.id) : undefined
                  }
                />
              ))}
            </section>
          );
        })}
        {archived.length > 0 && (
          <section>
            <button
              type="button"
              onClick={() => setHistoryOpen((v) => !v)}
              aria-expanded={historyOpen}
              className="unbutton flex h-7 w-full items-center gap-1 px-3 text-left font-sans text-caption font-semibold uppercase tracking-wide text-faint-foreground hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
            >
              <ChevronRight
                size={12}
                className={cn("transition-transform dur-1", historyOpen && "rotate-90")}
                aria-hidden
              />
              History
              <span className="font-code tabular-nums">{archived.length}</span>
            </button>
            {historyOpen &&
              archived.map((s) => (
                <SessionRow
                  key={s.id}
                  session={s}
                  state="stopped"
                  summary={summaries.get(s.id)}
                  lastActive={lastActive.get(s.id) ?? s.createdAtMs}
                  now={now}
                  active={false}
                  onActivate={() => resume(s.id)}
                  onRestore={() => resume(s.id)}
                  onClose={() => void closeSession(s)}
                />
              ))}
          </section>
        )}
        <section className="border-t border-border">
          <button
            type="button"
            onClick={() => setFilesOpen((v) => !v)}
            aria-expanded={filesOpen}
            className="unbutton flex h-7 w-full items-center gap-1 px-3 text-left font-sans text-caption font-semibold uppercase tracking-wide text-faint-foreground hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
          >
            <FolderTree size={12} aria-hidden />
            Files
          </button>
          {filesOpen && tree && (
            <div className="px-1 pb-2">
              <FileTree root={tree} />
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
