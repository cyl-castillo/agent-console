import { useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  ChevronRight,
  Copy,
  CopyPlus,
  FileText,
  MoreHorizontal,
  Pencil,
  Plus,
  RotateCcw,
  TerminalSquare,
  Trash2,
  X,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { AgentModelChooser } from "../../components/SessionList";
import { createSession } from "../../lib/createSession";
import { copyText } from "../../lib/clipboard";
import { useToastStore } from "../../stores/toastStore";
import { ContextMenu, Menu, type MenuItemDef } from "../../ui/Menu";
import { confirmDialog } from "../../stores/confirmStore";
import { useLiveStatusStore } from "../../stores/liveStatusStore";
import { useHandoffStore } from "../../stores/handoffStore";
import { useModelStore } from "../../stores/modelStore";
import { DEFAULT_AGENT } from "../../agents/profiles";
import { useProofStore } from "../../stores/proofStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { Badge, Chip } from "../../ui/Chip";
import { ciLabel, ciTone, prTermIds } from "../../lib/prInbox";
import { usePrStore } from "../../stores/prStore";
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
/// How long "Remove" can be undone before the session is really gone.
export const REMOVE_UNDO_MS = 5000;

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
  pr,
  lastActive,
  now,
  active,
  onActivate,
  onArchive,
  onRestore,
  onClose,
  onHandoff,
  renaming,
  onRenameStart,
  onRenameCommit,
  menu,
  tabIndex,
}: {
  session: TerminalSession;
  state: AgentState;
  summary?: string;
  cost?: number;
  /// F7: the PR this session's commits are on, with its CI word.
  pr?: { number: number; ci: string };
  lastActive: number;
  now: number;
  active: boolean;
  onActivate: () => void;
  onArchive?: () => void;
  onRestore?: () => void;
  onClose: () => void;
  onHandoff?: () => void;
  /// F6: the name is an input (double-click or F2 on the row).
  renaming: boolean;
  onRenameStart: () => void;
  onRenameCommit: (name: string | null) => void;
  /// F6: the row's actions, in the "…" button and on right-click.
  menu: MenuItemDef[];
  /// Roving: one row is the tab stop, ↑/↓ move between rows.
  tabIndex: 0 | -1;
}) {
  const [draft, setDraft] = useState(session.name);
  useEffect(() => {
    if (renaming) setDraft(session.name);
  }, [renaming, session.name]);
  return (
    <ContextMenu
      items={menu}
      className={cn(
        "group relative block transition-colors dur-1 hover:bg-[var(--hover)]",
        active && "bg-[var(--accent-12)]",
      )}
      data-testid={`session-${session.id}`}
      data-state={state}
    >
      <button
        type="button"
        onClick={onActivate}
        onDoubleClick={(e) => {
          e.preventDefault();
          onRenameStart();
        }}
        onKeyDown={(e) => {
          if (e.key === "F2") {
            e.preventDefault();
            onRenameStart();
          }
        }}
        aria-current={active ? "true" : undefined}
        tabIndex={tabIndex}
        data-session-row
        className="unbutton flex min-h-9 w-full items-center gap-2 px-3 py-1 text-left font-sans text-body text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
      >
        <StatusGlyph state={state} />
        <span className="min-w-0 flex-1">
          {renaming ? (
            <input
              aria-label={`Rename ${session.name}`}
              value={draft}
              autoFocus
              onFocus={(e) => e.currentTarget.select()}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => onRenameCommit(draft.trim() || null)}
              onClick={(e) => e.stopPropagation()}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") {
                  e.preventDefault();
                  onRenameCommit(draft.trim() || null);
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  onRenameCommit(null);
                }
              }}
              className="block w-full rounded-1 border border-border bg-background px-1 font-sans text-body text-foreground outline-none focus:border-primary"
              data-testid="session-rename"
            />
          ) : (
            <span className="block truncate" data-testid="session-name">
              {session.name}
            </span>
          )}
          {(session.worktree || summary || pr) && (
            <span className="block truncate text-small text-muted-foreground">
              {pr && (
                // Neutral text, coloured dot: a tinted chip over the active
                // row's wash fails contrast in the light theme (axe).
                <Chip
                  mono
                  className="mr-1 inline-flex items-center gap-1 align-middle"
                  data-testid="session-pr"
                  title={`PR #${pr.number} · ${ciLabel(pr.ci)}`}
                >
                  <span
                    aria-hidden
                    className={cn(
                      "inline-block size-1.5 rounded-full",
                      ciTone(pr.ci) === "ok" && "bg-ok",
                      ciTone(pr.ci) === "warn" && "bg-warn",
                      ciTone(pr.ci) === "danger" && "bg-danger",
                      ciTone(pr.ci) === "neutral" && "bg-muted-foreground",
                    )}
                  />
                  #{pr.number} · {ciLabel(pr.ci)}
                </Chip>
              )}
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
        <Tooltip
          content={
            session.status === "live" ? "Close — kills the process" : "Remove (undo for 5 s)"
          }
        >
          <IconButton
            size="sm"
            label={`Close ${session.name}`}
            icon={<X size={14} />}
            onClick={onClose}
          />
        </Tooltip>
        <Menu
          items={menu}
          trigger={
            <IconButton
              size="sm"
              label={`More actions for ${session.name}`}
              icon={<MoreHorizontal size={14} />}
              data-testid="session-menu"
            />
          }
        />
      </span>
    </ContextMenu>
  );
}

/// The Sessions list of the v2 shell: grouped by attention state, one line of
/// meaning per row, cost and age, a visible WIP count, History for archived
/// sessions. Rename in place, a menu per row, ↑/↓ between rows. The project
/// files live under Preview.
export function SessionsList() {
  const project = useSessionStore((s) => s.project);
  const { byState, lastActive, now, activeId } = useInbox();
  const summaries = useSummaries();
  const byTerm = useLiveStatusStore((s) => s.byTerm);
  const prOverview = usePrStore((s) => s.overview);
  // F7: the sessions whose commits are on the branch's PR carry its CI.
  const prTerms = useMemo(() => prTermIds(prOverview), [prOverview]);
  const prChip = (id: string) =>
    prOverview?.pr && prTerms.has(id)
      ? { number: prOverview.pr.number, ci: prOverview.pr.ci }
      : undefined;
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
  const rename = useTerminalsStore((s) => s.rename);
  const goTo = useShellStore((s) => s.goTo);
  const showToast = useToastStore((s) => s.show);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  // F6: "Remove" hides the row at once and really closes after the undo
  // window; Undo cancels the timer and the row comes back.
  const [removing, setRemoving] = useState<Set<string>>(() => new Set());
  const timers = useRef(new Map<string, number>());
  useEffect(() => {
    const t = timers.current;
    return () => t.forEach((id) => window.clearTimeout(id));
  }, []);
  const listRef = useRef<HTMLDivElement>(null);

  const live = sessions.filter((s) => s.status === "live" && !s.archived).length;
  const archived = sessions.filter((s) => s.archived && !removing.has(s.id));
  // The row that holds the tab stop: the active one, else the first shown.
  const stopId = useMemo(() => {
    const shown = GROUPS.flatMap(({ state }) => byState.get(state) ?? []).filter(
      (s) => !removing.has(s.id),
    );
    return shown.some((s) => s.id === activeId) ? activeId : (shown[0]?.id ?? null);
  }, [byState, activeId, removing]);

  const commitRename = (id: string, name: string | null) => {
    setRenaming(null);
    if (name) rename(id, name);
  };
  const removeSession = (s: TerminalSession) => {
    setRemoving((prev) => new Set(prev).add(s.id));
    const timer = window.setTimeout(() => {
      timers.current.delete(s.id);
      void close(s.id).finally(() =>
        setRemoving((prev) => {
          const next = new Set(prev);
          next.delete(s.id);
          return next;
        }),
      );
    }, REMOVE_UNDO_MS);
    timers.current.set(s.id, timer);
    showToast(`Removed "${s.name}"`, "info", {
      timeoutMs: REMOVE_UNDO_MS,
      action: {
        label: "Undo",
        onClick: () => {
          const t = timers.current.get(s.id);
          if (t !== undefined) {
            window.clearTimeout(t);
            timers.current.delete(s.id);
          }
          setRemoving((prev) => {
            const next = new Set(prev);
            next.delete(s.id);
            return next;
          });
        },
      },
    });
  };
  const openInTerminal = (s: TerminalSession) => {
    if (s.status === "stopped") resume(s.id);
    else setActive(s.id);
    goTo("sessions");
    if (!useShellStore.getState().drawerOpen) useShellStore.getState().toggleDrawer();
    window.setTimeout(() => window.dispatchEvent(new CustomEvent("ac:focus-terminal")), 50);
  };
  const duplicate = (s: TerminalSession) => {
    const { add, persist, setActive: activate } = useTerminalsStore.getState();
    const id = add(s.cwd, `${s.name} (copy)`, s.model, s.agent, s.worktree);
    activate(id);
    void persist();
  };
  const menuFor = (s: TerminalSession): MenuItemDef[] => [
    { label: "Rename", icon: <Pencil />, hint: "F2", onSelect: () => setRenaming(s.id) },
    { label: "Open in terminal", icon: <TerminalSquare />, onSelect: () => openInTerminal(s) },
    { label: "Duplicate", icon: <CopyPlus />, onSelect: () => duplicate(s) },
    {
      label: "Copy id",
      icon: <Copy />,
      onSelect: () =>
        void copyText(s.agentSessionId ?? s.id).then(
          () => showToast("Session id copied", "success"),
          (e) => showToast(`Copy failed: ${String(e)}`, "error"),
        ),
    },
    s.archived
      ? { label: "Restore", icon: <RotateCcw />, onSelect: () => resume(s.id) }
      : {
          label: "Archive",
          icon: <Archive />,
          disabled: s.status === "live",
          title: s.status === "live" ? "Close the session first" : undefined,
          onSelect: () => archive(s.id),
        },
    s.status === "live"
      ? {
          label: "Close",
          icon: <X />,
          danger: true,
          separatorBefore: true,
          onSelect: () => void closeSession(s),
        }
      : {
          label: "Remove",
          icon: <Trash2 />,
          danger: true,
          separatorBefore: true,
          hint: "undo 5 s",
          onSelect: () => removeSession(s),
        },
  ];
  // ↑/↓ move between rows (roving tab stop); Home/End jump.
  const onListKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    const rows = Array.from(
      listRef.current?.querySelectorAll<HTMLButtonElement>("[data-session-row]") ?? [],
    );
    if (rows.length === 0) return;
    const i = rows.findIndex((r) => r === document.activeElement);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? rows.length - 1
          : e.key === "ArrowDown"
            ? Math.min(rows.length - 1, i + 1)
            : Math.max(0, i - 1);
    e.preventDefault();
    rows[next]?.focus();
  };
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
      <div className="min-h-0 flex-1 overflow-auto" ref={listRef} onKeyDown={onListKeyDown}>
        {GROUPS.map(({ state, title }) => {
          const list = (byState.get(state) ?? []).filter((s) => !removing.has(s.id));
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
                  pr={prChip(s.id)}
                  lastActive={lastActive.get(s.id) ?? s.createdAtMs}
                  now={now}
                  active={s.id === activeId}
                  onActivate={() => (s.status === "stopped" ? resume(s.id) : setActive(s.id))}
                  // A live session is closed, not archived: archiving hides
                  // a process still running.
                  onArchive={s.status === "live" ? undefined : () => archive(s.id)}
                  onClose={() => (s.status === "live" ? void closeSession(s) : removeSession(s))}
                  onHandoff={
                    s.status === "live" && s.agent ? () => requestHandoff(s.id) : undefined
                  }
                  renaming={renaming === s.id}
                  onRenameStart={() => setRenaming(s.id)}
                  onRenameCommit={(name) => commitRename(s.id, name)}
                  menu={menuFor(s)}
                  tabIndex={s.id === stopId ? 0 : -1}
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
                  onClose={() => removeSession(s)}
                  renaming={renaming === s.id}
                  onRenameStart={() => setRenaming(s.id)}
                  onRenameCommit={(name) => commitRename(s.id, name)}
                  menu={menuFor(s)}
                  tabIndex={-1}
                />
              ))}
          </section>
        )}
      </div>
    </div>
  );
}
