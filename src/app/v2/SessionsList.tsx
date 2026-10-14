import { useMemo, useState } from "react";
import { Archive, ChevronRight, FileText, FolderTree, Plus, X } from "lucide-react";
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
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { Badge } from "../../ui/Chip";
import { IconButton } from "../../ui/IconButton";
import { SectionHeader } from "../../ui/SectionHeader";
import { StatusGlyph, type AgentState } from "../../ui/StatusGlyph";
import { Tooltip } from "../../ui/Tooltip";
import { useInbox } from "./useInbox";

const GROUPS: { state: AgentState; title: string }[] = [
  { state: "needs-you", title: "Needs you" },
  { state: "working", title: "Working" },
  { state: "done", title: "Idle" },
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
  age,
  active,
  onActivate,
  onArchive,
  onClose,
  onHandoff,
}: {
  session: TerminalSession;
  state: AgentState;
  summary?: string;
  cost?: number;
  age: string;
  active: boolean;
  onActivate: () => void;
  onArchive: () => void;
  onClose: () => void;
  onHandoff?: () => void;
}) {
  return (
    <div
      className={cn(
        "group relative flex items-center pr-1 transition-colors dur-1 hover:bg-[var(--hover)]",
        active && "bg-[var(--accent-12)]",
      )}
      data-testid={`session-${session.id}`}
    >
      <button
        type="button"
        onClick={onActivate}
        aria-current={active ? "true" : undefined}
        className="flex min-h-9 min-w-0 flex-1 items-center gap-2 px-3 py-1 text-left font-sans text-body text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
      >
        <StatusGlyph state={state} />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-1.5">
            <span className="truncate">{session.name}</span>
            {session.worktree && (
              <span className="truncate font-code text-caption text-faint-foreground">
                {session.worktree.branch}
              </span>
            )}
          </span>
          {summary && (
            <span className="block truncate text-small text-muted-foreground">{summary}</span>
          )}
        </span>
        <span className="shrink-0 text-right font-code text-caption text-muted-foreground tabular-nums">
          {cost !== undefined && <span className="block">${cost.toFixed(2)}</span>}
          <span className="block">{age}</span>
        </span>
      </button>
      <span className="flex shrink-0 items-center opacity-0 transition-opacity dur-1 focus-within:opacity-100 group-hover:opacity-100">
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
        <Tooltip content="Archive — hide in History, resumable anytime">
          <IconButton
            size="sm"
            label={`Archive ${session.name}`}
            icon={<Archive size={14} />}
            onClick={onArchive}
          />
        </Tooltip>
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
  const { byState, now, activeId } = useInbox();
  const summaries = useSummaries();
  const byTerm = useLiveStatusStore((s) => s.byTerm);
  const sessions = useTerminalsStore((s) => s.sessions);
  const setActive = useTerminalsStore((s) => s.setActive);
  const archive = useTerminalsStore((s) => s.archive);
  const resume = useTerminalsStore((s) => s.resume);
  const close = useTerminalsStore((s) => s.close);
  const requestHandoff = useHandoffStore((s) => s.request);
  const defaultAgentFor = useModelStore((s) => s.defaultAgentFor);
  const [choosing, setChoosing] = useState(false);
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
              : `${live} live`
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
        <Tooltip content="New session — pick an agent and model">
          <IconButton
            size="sm"
            label="New session"
            icon={<Plus size={14} />}
            active={choosing}
            onClick={() => setChoosing((v) => !v)}
            data-testid="sessions-new"
          />
        </Tooltip>
      </div>
      {choosing && project && (
        <div className="border-b border-border px-2 pb-2">
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
            <section key={state}>
              <SectionHeader title={title} count={list.length} />
              {list.map((s) => (
                <SessionRow
                  key={s.id}
                  session={s}
                  state={state}
                  summary={summaries.get(s.id)}
                  cost={byTerm[s.id]?.costUsd}
                  age={relative(s.lastActiveMs ?? s.createdAtMs, now)}
                  active={s.id === activeId}
                  onActivate={() => (s.status === "stopped" ? resume(s.id) : setActive(s.id))}
                  onArchive={() => archive(s.id)}
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
              className="flex h-7 w-full items-center gap-1 px-3 text-left font-sans text-caption font-semibold uppercase tracking-wide text-faint-foreground hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
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
                  age={relative(s.lastActiveMs ?? s.createdAtMs, now)}
                  active={false}
                  onActivate={() => resume(s.id)}
                  onArchive={() => resume(s.id)}
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
            className="flex h-7 w-full items-center gap-1 px-3 text-left font-sans text-caption font-semibold uppercase tracking-wide text-faint-foreground hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
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
