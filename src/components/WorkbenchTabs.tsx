import { useMemo } from "react";

import { useSkillsStore } from "../stores/skillsStore";
import { usePermissionsStore } from "../stores/permissionsStore";
import { parseRaw, classify } from "../permissions/rules";

export type WorkbenchTab = "skills" | "permissions";

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
    <div className="workbench-tabs">
      <TabButton
        label="Skills"
        count={skillsCount}
        active={active === "skills"}
        onClick={() => onChange("skills")}
      />
      <TabButton
        label="Permissions"
        count={permsCount}
        flagged={flagged}
        active={active === "permissions"}
        onClick={() => onChange("permissions")}
      />
    </div>
  );
}

function TabButton({ label, count, flagged, active, onClick }: {
  label: string;
  count?: number;
  flagged?: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      className={`wb-tab ${active ? "active" : ""}`}
      onClick={onClick}
      title={flagged ? `${flagged} rule${flagged === 1 ? "" : "s"} need review` : undefined}
    >
      <span className="wb-tab-label">{label}</span>
      {count !== undefined && count > 0 && <span className="wb-tab-count">{count}</span>}
      {flagged !== undefined && flagged > 0 && (
        <span className="wb-tab-flag" title={`${flagged} broad/dangerous allow rule${flagged === 1 ? "" : "s"}`}>
          {flagged}
        </span>
      )}
    </button>
  );
}
