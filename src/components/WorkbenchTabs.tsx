import { useMemo } from "react";

import { useSkillsStore } from "../stores/skillsStore";
import { usePermissionsStore } from "../stores/permissionsStore";
import { useAdvisorStore } from "../stores/advisorStore";
import { useVaultStore } from "../stores/vaultStore";
import { useContextStore } from "../stores/contextStore";
import { useFeedbackStore } from "../stores/feedbackStore";
import { usePluginsStore } from "../stores/pluginsStore";
import { useMcpStore } from "../stores/mcpStore";
import { parseRaw, classify } from "../permissions/rules";
import { Icon, type IconName } from "./Icon";

export type WorkbenchTab = "skills" | "permissions" | "advisor" | "vault" | "context" | "plugins" | "mcp" | "feedback";

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
  const vaultCount = useVaultStore((s) => s.entries.length);
  const memoriesCount = useContextStore((s) => s.memories.length);
  const pluginsCount = usePluginsStore((s) => s.installed.length);
  const mcpCount = useMcpStore((s) => s.servers.length);
  const feedbackEnabled = useFeedbackStore((s) => s.devEnabled === true);

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

  return (
    <div className="workbench-strip">
      <StripButton
        icon="puzzle"
        label="Skills"
        title="Skills"
        count={skillsCount}
        active={active === "skills"}
        onClick={() => onChange("skills")}
      />
      <StripButton
        icon="key"
        label="Perms"
        title="Permissions"
        count={permsCount}
        flagged={flagged}
        active={active === "permissions"}
        onClick={() => onChange("permissions")}
      />
      <StripButton
        icon="lightbulb"
        label={advisorAnalyzing ? "…" : "Advisor"}
        title={advisorAnalyzing ? "Advisor (analyzing)" : "Advisor"}
        count={advisorPending}
        active={active === "advisor"}
        onClick={() => onChange("advisor")}
      />
      <StripButton
        icon="lock"
        label="Vault"
        title="Vault"
        count={vaultCount}
        active={active === "vault"}
        onClick={() => onChange("vault")}
      />
      <StripButton
        icon="file-text"
        label="Context"
        title="Context"
        count={memoriesCount}
        active={active === "context"}
        onClick={() => onChange("context")}
      />
      <StripButton
        icon="plug"
        label="Plugins"
        title="Plugins"
        count={pluginsCount}
        active={active === "plugins"}
        onClick={() => onChange("plugins")}
      />
      <StripButton
        icon="server"
        label="MCP"
        title="MCP servers"
        count={mcpCount}
        active={active === "mcp"}
        onClick={() => onChange("mcp")}
      />
      {feedbackEnabled && (
        <StripButton
          icon="message-square"
          label="Feedback"
          title="Feedback (dev only)"
          active={active === "feedback"}
          onClick={() => onChange("feedback")}
        />
      )}
    </div>
  );
}

function StripButton({ icon, label, title, count, flagged, active, onClick }: {
  icon: IconName;
  label: string;
  title: string;
  count?: number;
  flagged?: number;
  active: boolean;
  onClick: () => void;
}) {
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
