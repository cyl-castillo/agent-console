import { useMemo } from "react";

import { useSkillsStore } from "../stores/skillsStore";
import { usePermissionsStore } from "../stores/permissionsStore";
import { useAdvisorStore } from "../stores/advisorStore";
import { useLearningStore } from "../stores/learningStore";
import { useVaultStore } from "../stores/vaultStore";
import { useContextStore } from "../stores/contextStore";
import { usePluginsStore } from "../stores/pluginsStore";
import { useJiraStore } from "../stores/jiraStore";
import { useNotesStore } from "../stores/notesStore";
import { useMcpStore } from "../stores/mcpStore";
import { useRoundtableStore } from "../stores/roundtableStore";
import { useSchedulerStore } from "../stores/schedulerStore";
import { parseRaw, classify } from "../permissions/rules";
import { Icon, type IconName } from "./Icon";

// Single source of truth for tab ids and their strip grouping lives in
// lib/workbenchTabs (a component file exporting constants would break fast
// refresh). Re-exporting the type keeps existing `import type` sites working.
import {
  WORKBENCH_GROUPS,
  groupForTab,
  type WorkbenchGroupKey,
  type WorkbenchTab,
} from "../lib/workbenchTabs";
export type { WorkbenchTab } from "../lib/workbenchTabs";

/// Allow-rules that classify as broad/dangerous — surfaced on the Trust
/// group button and again on its Permissions sub-tab.
function useFlaggedRules(): number {
  const permsRules = usePermissionsStore((s) => s.snapshot?.rules);
  return useMemo(() => {
    if (!permsRules) return 0;
    return permsRules.reduce((n, r) => {
      if (r.effect !== "allow") return n;
      const p = parseRaw(r.raw);
      if (!p) return n;
      const risk = classify({
        scope: r.scope,
        effect: r.effect,
        tool: p.tool,
        pattern: p.pattern,
        raw: r.raw,
      }).risk;
      return risk === "broad" || risk === "dangerous" ? n + 1 : n;
    }, 0);
  }, [permsRules]);
}

interface ButtonMeta {
  icon: IconName;
  label: string;
  title: string;
  count?: number;
  flagged?: number;
}

export function WorkbenchTabs({
  active,
  onChange,
}: {
  active: WorkbenchTab;
  onChange: (tab: WorkbenchTab) => void;
}) {
  const jiraCount = useJiraStore((s) => s.issues.length);
  const notesCount = useNotesStore((s) => s.notes.length);
  const memoriesCount = useContextStore((s) => s.memories.length);
  const advisorPending = useAdvisorStore(
    (s) => s.items.filter((it) => it.status === "proposed" || it.status === "error").length,
  );
  const advisorAnalyzing = useAdvisorStore((s) => s.status === "analyzing");
  const learningPending = useLearningStore(
    (s) => s.items.filter((it) => it.status === "proposed" || it.status === "error").length,
  );
  const learningReflecting = useLearningStore((s) => s.status === "reflecting");
  const rtActive = useRoundtableStore((s) => s.phase === "running" || s.phase === "paused");
  const scheduledCount = useSchedulerStore((s) => s.jobs.filter((j) => j.enabled).length);
  const schedulerRunning = useSchedulerStore((s) => s.runningJobIds.length > 0);
  const pluginsCount = usePluginsStore((s) => s.installed.length);
  const mcpCount = useMcpStore((s) => s.servers.length);
  const flagged = useFlaggedRules();

  const coachBusy = advisorAnalyzing || learningReflecting;

  // One button per group. Badges are actionable where possible: Coach shows
  // pending suggestions (not installed skills), Trust shows only the flag.
  const meta: Record<WorkbenchGroupKey, ButtonMeta> = {
    tasks: {
      icon: "check",
      label: "Tasks",
      title: "Tasks — your Jira queue + agenda",
      count: jiraCount,
    },
    notes: {
      icon: "sticky-note",
      label: "Notes",
      title: "Notes — your per-project scratchpad",
      count: notesCount,
    },
    proof: {
      icon: "shield-check",
      label: "Proof",
      title:
        "Proof — attach a verifiable proof packet to your next PR: what was asked, what you approved, what changed. Verified in any browser, no install.",
    },
    context: {
      icon: "file-text",
      label: "Context",
      title: "Context — CLAUDE.md & memories",
      count: memoriesCount,
    },
    coach: {
      icon: "lightbulb",
      label: coachBusy ? "Coach…" : "Coach",
      title: coachBusy
        ? "Coach (working)"
        : "Coach — the agent's playbook: skills, advisor suggestions, learning",
      count: advisorPending + learningPending,
    },
    room: {
      icon: "users",
      label: rtActive ? "Room…" : "Room",
      title: rtActive ? "Room (running)" : "Agent Room — you + N agents converse on a problem",
    },
    schedule: {
      icon: "clock",
      label: schedulerRunning ? "Sched…" : "Schedule",
      title: schedulerRunning
        ? "Schedule (a job is running)"
        : "Schedule — run skills/prompts/pipelines on a clock (suggest-only)",
      count: scheduledCount,
    },
    trust: {
      icon: "key",
      label: "Trust",
      title: "Trust — permissions + vault: what the agent can touch",
      flagged,
    },
    addons: {
      icon: "plug",
      label: "Add-ons",
      title: "Add-ons — plugins + MCP servers",
      count: pluginsCount + mcpCount,
    },
  };

  const sections: { label: string; groups: WorkbenchGroupKey[] }[] = [
    { label: "Work", groups: ["tasks", "notes", "proof", "context"] },
    { label: "Agents", groups: ["coach", "room", "schedule"] },
    { label: "Config", groups: ["trust", "addons"] },
  ];

  return (
    <div className="workbench-strip">
      {sections.map((sec) => (
        <div className="wb-strip-group" key={sec.label}>
          <div className="wb-strip-group-label" aria-hidden="true">
            {sec.label}
          </div>
          {sec.groups.map((key) => {
            const group = WORKBENCH_GROUPS.find((g) => g.key === key)!;
            const isActive = (group.tabs as readonly string[]).includes(active);
            return (
              <StripButton
                key={key}
                {...meta[key]}
                active={isActive}
                onClick={() => {
                  // Re-clicking an active group keeps the current sub-tab.
                  if (!isActive) onChange(group.tabs[0]);
                }}
              />
            );
          })}
        </div>
      ))}
    </div>
  );
}

/// Sub-switcher rendered at the top of the workbench content when the active
/// tab belongs to a merged group (Tasks, Coach, Trust, Add-ons). Groupless
/// tabs (transfer, feedback) and single-tab groups render nothing.
export function WorkbenchSubTabs({
  active,
  onChange,
}: {
  active: WorkbenchTab;
  onChange: (tab: WorkbenchTab) => void;
}) {
  const jiraCount = useJiraStore((s) => s.issues.length);
  const skillsCount = useSkillsStore((s) => s.installed.length);
  const advisorPending = useAdvisorStore(
    (s) => s.items.filter((it) => it.status === "proposed" || it.status === "error").length,
  );
  const advisorAnalyzing = useAdvisorStore((s) => s.status === "analyzing");
  const learningPending = useLearningStore(
    (s) => s.items.filter((it) => it.status === "proposed" || it.status === "error").length,
  );
  const learningReflecting = useLearningStore((s) => s.status === "reflecting");
  const permsCount = usePermissionsStore((s) => s.snapshot?.rules.length ?? 0);
  const vaultCount = useVaultStore((s) => s.entries.length);
  const pluginsCount = usePluginsStore((s) => s.installed.length);
  const mcpCount = useMcpStore((s) => s.servers.length);
  const flagged = useFlaggedRules();

  const group = groupForTab(active);
  if (!group || group.tabs.length < 2) return null;

  const meta: Partial<Record<WorkbenchTab, { label: string; count?: number; flagged?: number }>> = {
    jira: { label: "Queue", count: jiraCount },
    agenda: { label: "Agenda" },
    skills: { label: "Skills", count: skillsCount },
    advisor: { label: advisorAnalyzing ? "Advisor…" : "Advisor", count: advisorPending },
    learning: { label: learningReflecting ? "Learning…" : "Learning", count: learningPending },
    permissions: { label: "Permissions", count: permsCount, flagged },
    vault: { label: "Vault", count: vaultCount },
    plugins: { label: "Plugins", count: pluginsCount },
    mcp: { label: "MCP", count: mcpCount },
  };

  return (
    <div className="wb-subtabs" role="tablist" aria-label="Section tabs">
      {group.tabs.map((t) => {
        const m = meta[t] ?? { label: t };
        return (
          <button
            key={t}
            role="tab"
            aria-selected={active === t}
            className={`wb-subtab ${active === t ? "active" : ""}`}
            onClick={() => onChange(t)}
          >
            {m.label}
            {m.count !== undefined && m.count > 0 && (
              <span className="wb-subtab-count">{m.count}</span>
            )}
            {m.flagged !== undefined && m.flagged > 0 && (
              <span className="wb-subtab-flag">{m.flagged}</span>
            )}
          </button>
        );
      })}
    </div>
  );
}

interface StripProps extends ButtonMeta {
  active: boolean;
  onClick: () => void;
}

function StripButton({ icon, label, title, count, flagged, active, onClick }: StripProps) {
  const tooltip = flagged
    ? `${title} — ${flagged} rule${flagged === 1 ? "" : "s"} need review`
    : title;
  return (
    <button className={`wb-strip-btn ${active ? "active" : ""}`} onClick={onClick} title={tooltip}>
      <span className="wb-strip-icon">
        <Icon name={icon} size={18} />
      </span>
      <span className="wb-strip-label">{label}</span>
      {count !== undefined && count > 0 && <span className="wb-strip-count">{count}</span>}
      {flagged !== undefined && flagged > 0 && <span className="wb-strip-flag">{flagged}</span>}
    </button>
  );
}
