import { useEffect } from "react";
import { GitBranch, Info } from "lucide-react";
import {
  AgentStatePill,
  HooksPill,
  MemoryPill,
  ModelPill,
  UsagePill,
  VoicePill,
} from "../../components/StatusBar";
import { useChangesStore } from "../../stores/changesStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useShellStore } from "../../stores/shellStore";
import { useTerminalsStore } from "../../stores/terminalsStore";
import { useUIStore } from "../../stores/uiStore";
import type { WorkspaceContext } from "../../types/domain";
import { Badge } from "../../ui/Chip";
import { IconButton } from "../../ui/IconButton";
import { Popover } from "../../ui/Popover";
import { Tooltip } from "../../ui/Tooltip";
import { useInbox } from "./useInbox";
import { capFor, useBudgetStore } from "../../stores/budgetStore";
import { useLiveStatusStore } from "../../stores/liveStatusStore";
import { BudgetRing } from "../../ui/BudgetRing";
import { Input } from "../../ui/Field";

declare const __APP_VERSION__: string;

/// Four signals instead of fourteen: agent state, decisions pending, branch +
/// changes, hooks health. Everything else (model, usage, memory, voice,
/// language, framework, files, version) lives in the details popover.
export function StatusBarV2({ workspace }: { workspace?: WorkspaceContext | null }) {
  const project = useSessionStore((s) => s.project);
  const status = useChangesStore((s) => s.status);
  const branches = useChangesStore((s) => s.branches);
  const loadBranches = useChangesStore((s) => s.loadBranches);
  const { decisions: pending } = useInbox();
  const budgets = useBudgetStore();
  const byTerm = useLiveStatusStore((s) => s.byTerm);
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const setCenterTab = useUIStore((s) => s.setTab);
  const goTo = useShellStore((s) => s.goTo);
  const branch = status?.branch ?? null;
  useEffect(() => {
    if (branch && branches.length === 0) void loadBranches();
  }, [branch, branches.length, loadBranches]);
  if (!project) return null;

  const info = branches.find((b) => b.name === branch);
  const changes = status?.changes.length ?? 0;
  const active = sessions.find((s) => s.id === activeId);
  const live = sessions.filter((s) => s.status === "live").length;

  return (
    <footer
      className="statusbar flex h-6 items-center gap-1 border-t border-border bg-surface-1 px-2 font-sans text-caption text-muted-foreground"
      data-testid="statusbar-v2"
    >
      <AgentStatePill
        onShowTerminal={() => {
          goTo("sessions");
          setCenterTab("terminal");
        }}
      />
      <Tooltip content="Decisions waiting for you">
        <button
          type="button"
          onClick={() => goTo("inbox")}
          className="inline-flex h-5 items-center gap-1 rounded-1 px-1.5 hover:bg-[var(--hover)] hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-1 focus-visible:outline-ring"
          data-testid="sb-decisions"
        >
          <span>decisions</span>
          <Badge count={pending} className="bg-warn text-background" />
          {pending === 0 && <span className="font-code tabular-nums">0</span>}
        </button>
      </Tooltip>
      <span className="flex-1" />
      {branch && (
        <Tooltip content="Open Changes">
          <button
            type="button"
            onClick={() => {
              goTo("sessions");
              setCenterTab("changes");
            }}
            className="inline-flex h-5 items-center gap-1 rounded-1 px-1.5 font-code hover:bg-[var(--hover)] hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-1 focus-visible:outline-ring"
            data-testid="sb-branch"
          >
            <GitBranch size={12} aria-hidden />
            <span>{branch}</span>
            {info && (info.ahead > 0 || info.behind > 0) && (
              <span className="text-faint-foreground">
                {info.ahead > 0 && <>↑{info.ahead}</>}
                {info.behind > 0 && <>↓{info.behind}</>}
              </span>
            )}
            {changes > 0 && <span className="text-warn">· {changes} changed</span>}
          </button>
        </Tooltip>
      )}
      <HooksPill />
      <Popover
        trigger={<IconButton size="sm" label="Session details" icon={<Info size={14} />} />}
        title={active ? active.name : project.name}
        side="top"
        align="end"
      >
        <div className="flex flex-col gap-1 [&_.sb-item]:px-0">
          {active && <ModelPill session={active} projectRoot={project.root} />}
          {active?.agentSessionId && (
            <UsagePill
              termId={active.id}
              sessionId={active.agentSessionId}
              projectRoot={project.root}
              agent={active.agent}
              live={active.status === "live"}
            />
          )}
          {active && <MemoryPill termId={active.id} />}
          {active && (
            <div className="mt-1 flex items-center gap-2 text-caption" data-testid="budget-row">
              <BudgetRing
                spent={byTerm[active.id]?.costUsd ?? 0}
                cap={capFor(budgets, active.id)}
              />
              <label className="ml-auto flex items-center gap-1 text-faint-foreground">
                cap $
                <Input
                  type="number"
                  min={0}
                  step={1}
                  aria-label="Session budget in USD (0 = none)"
                  className="h-5 w-16 px-1 text-caption"
                  value={budgets.caps[active.id] ?? ""}
                  placeholder={budgets.defaultCap ? String(budgets.defaultCap) : "none"}
                  onChange={(e) => budgets.setCap(active.id, Number(e.target.value) || 0)}
                />
              </label>
            </div>
          )}
          <VoicePill />
          <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-caption text-faint-foreground">
            <span>{live} live</span>
            {project.language && <span>{project.language}</span>}
            {project.framework && <span>{project.framework}</span>}
            {workspace?.fileCount ? <span>{workspace.fileCount} files</span> : null}
            <span>v{__APP_VERSION__}</span>
          </div>
        </div>
      </Popover>
    </footer>
  );
}
