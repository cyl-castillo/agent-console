import { useMemo } from "react";

import { useSkillsStore } from "../stores/skillsStore";
import { usePermissionsStore } from "../stores/permissionsStore";
import { useAdvisorStore } from "../stores/advisorStore";
import { useLearningStore } from "../stores/learningStore";
import { useVaultStore } from "../stores/vaultStore";
import { useContextStore } from "../stores/contextStore";
import { useFeedbackStore } from "../stores/feedbackStore";
import { usePluginsStore } from "../stores/pluginsStore";
import { useJiraStore } from "../stores/jiraStore";
import { useMcpStore } from "../stores/mcpStore";
import { useRoundtableStore } from "../stores/roundtableStore";
import { useSchedulerStore } from "../stores/schedulerStore";
import { parseRaw, classify } from "../permissions/rules";
import { Icon, type IconName } from "./Icon";

export type WorkbenchTab = "skills" | "permissions" | "advisor" | "learning" | "roundtable" | "schedule" | "vault" | "context" | "plugins" | "mcp" | "transfer" | "feedback" | "jira" | "agenda";

export function WorkbenchTabs({
  active,
  onChange,
}: {
  active: WorkbenchTab;
  onChange: (tab: WorkbenchTab) => void;
}) {
  const skillsCount = useSkillsStore((s) => s.installed.length);
  const permsCount = usePermissionsStore((s) => s.snapshot?.rules.length ?? 0);
  const permsRules = usePermissionsStore((s) => s.snapshot?.rules);
  const advisorPending = useAdvisorStore((s) =>
    s.items.filter((it) => it.status === "proposed" || it.status === "error").length,
  );
  const advisorAnalyzing = useAdvisorStore((s) => s.status === "analyzing");
  const learningPending = useLearningStore((s) =>
    s.items.filter((it) => it.status === "proposed" || it.status === "error").length,
  );
  const learningReflecting = useLearningStore((s) => s.status === "reflecting");
  const vaultCount = useVaultStore((s) => s.entries.length);
  const memoriesCount = useContextStore((s) => s.memories.length);
  const pluginsCount = usePluginsStore((s) => s.installed.length);
  const jiraCount = useJiraStore((s) => s.issues.length);
  const mcpCount = useMcpStore((s) => s.servers.length);
  const feedbackEnabled = useFeedbackStore((s) => s.devEnabled === true);
  const rtActive = useRoundtableStore(
    (s) => s.phase === "running" || s.phase === "paused",
  );
  const scheduledCount = useSchedulerStore((s) => s.jobs.filter((j) => j.enabled).length);
  const schedulerRunning = useSchedulerStore((s) => s.runningJobIds.length > 0);

  const flagged = useMemo(() => {
    if (!permsRules) return 0;
    return permsRules.reduce((n, r) => {
      if (r.effect !== "allow") return n;
      const p = parseRaw(r.raw);
      if (!p) return n;
      const risk = classify({
        scope: r.scope, effect: r.effect, tool: p.tool, pattern: p.pattern, raw: r.raw,
      }).risk;
      return risk === "broad" || risk === "dangerous" ? n + 1 : n;
    }, 0);
  }, [permsRules]);

  // Descriptor per tab so the strip can be rendered in labeled groups (below)
  // rather than as one flat wall of 11 equal buttons.
  const tabs: Record<WorkbenchTab, Omit<StripProps, "active" | "onClick">> = {
    skills: { icon: "puzzle", label: "Skills", title: "Skills", count: skillsCount },
    jira: { icon: "check", label: "Tasks", title: "Tasks — your assigned Jira issues", count: jiraCount },
    agenda: { icon: "calendar", label: "Agenda", title: "Agenda — task due dates + scheduled jobs" },
    context: { icon: "file-text", label: "Context", title: "Context", count: memoriesCount },
    advisor: {
      icon: "lightbulb",
      label: advisorAnalyzing ? "…" : "Advisor",
      title: advisorAnalyzing ? "Advisor (analyzing)" : "Advisor",
      count: advisorPending,
    },
    learning: {
      icon: "sparkles",
      label: learningReflecting ? "…" : "Learning",
      title: learningReflecting ? "Learning (reflecting)" : "Learning — reflect on your activity and suggest improvements",
      count: learningPending,
    },
    roundtable: {
      icon: "users",
      label: rtActive ? "Room…" : "Room",
      title: rtActive ? "Room (running)" : "Agent Room — you + N agents converse on a problem",
    },
    schedule: {
      icon: "clock",
      label: schedulerRunning ? "Sched…" : "Schedule",
      title: schedulerRunning ? "Schedule (a job is running)" : "Schedule — run skills/prompts/pipelines on a clock (suggest-only)",
      count: scheduledCount,
    },
    permissions: { icon: "key", label: "Perms", title: "Permissions", count: permsCount, flagged },
    vault: { icon: "lock", label: "Vault", title: "Vault", count: vaultCount },
    plugins: { icon: "plug", label: "Plugins", title: "Plugins", count: pluginsCount },
    mcp: { icon: "server", label: "MCP", title: "MCP servers", count: mcpCount },
    transfer: {
      icon: "archive",
      label: "Transfer",
      title: "Export / Import — move your work to or from another installation",
    },
    feedback: { icon: "message-square", label: "Feedback", title: "Feedback (dev only)" },
  };

  const groups: { label: string; tabs: WorkbenchTab[] }[] = [
    { label: "Workspace", tabs: ["skills", "jira", "agenda", "context", "advisor", "learning"] },
    { label: "Agents", tabs: ["roundtable", "schedule"] },
    { label: "Config", tabs: ["permissions", "vault", "plugins", "mcp", "transfer"] },
  ];
  if (feedbackEnabled) groups.push({ label: "Dev", tabs: ["feedback"] });

  return (
    <div className="workbench-strip">
      {groups.map((g) => (
        <div className="wb-strip-group" key={g.label}>
          <div className="wb-strip-group-label" aria-hidden="true">{g.label}</div>
          {g.tabs.map((t) => (
            <StripButton
              key={t}
              {...tabs[t]}
              active={active === t}
              onClick={() => onChange(t)}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

interface StripProps {
  icon: IconName;
  label: string;
  title: string;
  count?: number;
  flagged?: number;
  active: boolean;
  onClick: () => void;
}

function StripButton({ icon, label, title, count, flagged, active, onClick }: StripProps) {
  const tooltip = flagged
    ? `${title} — ${flagged} rule${flagged === 1 ? "" : "s"} need review`
    : title;
  return (
    <button
      className={`wb-strip-btn ${active ? "active" : ""}`}
      onClick={onClick}
      title={tooltip}
    >
      <span className="wb-strip-icon"><Icon name={icon} size={18} /></span>
      <span className="wb-strip-label">{label}</span>
      {count !== undefined && count > 0 && <span className="wb-strip-count">{count}</span>}
      {flagged !== undefined && flagged > 0 && <span className="wb-strip-flag">{flagged}</span>}
    </button>
  );
}
