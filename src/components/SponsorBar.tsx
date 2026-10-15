import { useEffect, useState } from "react";
import { useLiveStatusStore } from "../stores/liveStatusStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSponsorStore, weekSpend } from "../stores/sponsorStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { BudgetRing } from "../ui/BudgetRing";
import { Chip } from "../ui/Chip";
import { Field, Input } from "../ui/Field";
import { Select } from "../ui/Select";
import { Switch } from "../ui/Switch";
import type { RiskLevel } from "../types/domain";

/// D11: the sponsor of this product. Objective, weekly budget, the risk
/// threshold the console may decide alone up to, and the switch. What the
/// policy decides is witnessed as `approval_decision` by `sponsor`; the
/// human only sees what crosses the threshold.
export function SponsorBar() {
  const projectRoot = useSessionStore((s) => s.project?.root);
  const sponsor = useSponsorStore((s) => s.sponsor);
  const autoApproved = useSponsorStore((s) => s.autoApproved);
  const load = useSponsorStore((s) => s.load);
  const save = useSponsorStore((s) => s.save);
  const sessions = useTerminalsStore((s) => s.sessions);
  const byTerm = useLiveStatusStore((s) => s.byTerm);
  const [objective, setObjective] = useState("");
  const [owner, setOwner] = useState("");
  const [budget, setBudget] = useState("");

  useEffect(() => {
    if (projectRoot) void load(projectRoot);
  }, [projectRoot, load]);
  useEffect(() => {
    setObjective(sponsor?.objective ?? "");
    setOwner(sponsor?.owner ?? "");
    setBudget(sponsor?.weeklyBudgetUsd ? String(sponsor.weeklyBudgetUsd) : "");
  }, [sponsor]);

  const spent = weekSpend(sessions, byTerm);
  const cap = sponsor?.weeklyBudgetUsd ?? 0;
  const paused = Boolean(sponsor?.autoApprove && cap > 0 && spent >= cap);

  return (
    <section
      className="mb-3 flex flex-col gap-2 rounded-2 border border-border bg-surface-1 p-3 font-sans"
      aria-label="Sponsor"
      data-testid="sponsor-bar"
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-small font-semibold text-foreground">Sponsor</span>
        <Chip tone={sponsor?.autoApprove ? (paused ? "warn" : "ok") : "neutral"} mono>
          {sponsor?.autoApprove
            ? paused
              ? "auto-approval paused: budget reached"
              : `auto-approves ≤ ${sponsor.riskThreshold}`
            : "decides nothing alone"}
        </Chip>
        {cap > 0 && (
          <span className="flex items-center gap-1 text-caption text-muted-foreground">
            <BudgetRing spent={spent} cap={cap} />
            this week
          </span>
        )}
        <span
          className="ml-auto text-caption text-faint-foreground"
          data-testid="sponsor-auto-count"
        >
          {autoApproved} decided by the policy this week
        </span>
      </div>
      <div className="grid grid-cols-1 gap-2 md:grid-cols-[2fr_1fr_1fr_1fr_auto]">
        <Field label="Objective this week">
          <Input
            value={objective}
            placeholder="What this product is trying to achieve"
            onChange={(e) => setObjective(e.target.value)}
            onBlur={() => objective !== (sponsor?.objective ?? "") && void save({ objective })}
          />
        </Field>
        <Field label="Owner">
          <Input
            value={owner}
            placeholder="who answers"
            onChange={(e) => setOwner(e.target.value)}
            onBlur={() => owner !== (sponsor?.owner ?? "") && void save({ owner })}
          />
        </Field>
        <Field label="Weekly budget (USD, 0 = none)">
          <Input
            type="number"
            min={0}
            step={1}
            value={budget}
            placeholder="0"
            onChange={(e) => setBudget(e.target.value)}
            onBlur={() => {
              const n = Math.max(0, Number(budget) || 0);
              if (n !== (sponsor?.weeklyBudgetUsd ?? 0)) void save({ weeklyBudgetUsd: n });
            }}
          />
        </Field>
        <Field label="Decides alone up to">
          <Select<RiskLevel>
            label="Risk threshold"
            value={sponsor?.riskThreshold ?? "low"}
            onValueChange={(v) => void save({ riskThreshold: v })}
            options={[
              { value: "low", label: "low (read-only, tests)" },
              { value: "medium", label: "medium (edits, builds)" },
              { value: "high", label: "high (irreversible) — not advised" },
            ]}
          />
        </Field>
        <div className="flex items-end pb-1">
          <Switch
            label="Auto-approve"
            checked={Boolean(sponsor?.autoApprove)}
            onCheckedChange={(v) => void save({ autoApprove: v })}
          />
        </div>
      </div>
    </section>
  );
}
