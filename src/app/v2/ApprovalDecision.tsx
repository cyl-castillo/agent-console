import { useMemo, useState } from "react";
import { AlwaysPanel } from "../../components/ApprovalModal";
import { commitAlways, decideApproval } from "../../lib/approvalActions";
import { describe, editPreview } from "../../lib/approvalPreview";
import { tierFor } from "../../lib/decisions";
import { resolveApprovalEngine, suggestRules } from "../../permissions/rules";
import type { Scope } from "../../permissions/types";
import { useTerminalsStore } from "../../stores/terminalsStore";
import type { ApprovalRequest } from "../../types/domain";
import { Chip } from "../../ui/Chip";
import { DecisionCard } from "../../ui/DecisionCard";

/// A pending approval as an Inbox card. Same decisions as the modal (deny /
/// always / approve once), same rule store; the card adds the tier, the
/// preview the tier demands and, for high tier, the typed word.
export function ApprovalDecision({
  req,
  index,
  total,
}: {
  req: ApprovalRequest;
  index: number;
  total: number;
}) {
  const sessions = useTerminalsStore((s) => s.sessions);
  const session = sessions.find((s) => s.id === req.termId);
  const engine = useMemo(() => resolveApprovalEngine(req, sessions), [req, sessions]);
  const [busy, setBusy] = useState(false);
  const [always, setAlways] = useState(false);
  const [scope, setScope] = useState<Scope>("project");
  const suggestions = useMemo(() => suggestRules(req, scope), [req, scope]);
  const info = tierFor(req, { allowedHosts: session?.allowedHosts });
  const { primary, secondary } = describe(req);
  const diff = editPreview(req);

  const run = (p: Promise<boolean>) => {
    setBusy(true);
    void p.then((ok) => {
      if (!ok) setBusy(false);
    });
  };

  const preview = diff ? (
    <pre className="m-0 max-h-64 overflow-auto p-2 font-code text-small leading-snug">
      {diff.slice(0, 80).map((l, i) => (
        <div
          key={i}
          className={
            l.type === "add"
              ? "text-ok"
              : l.type === "del"
                ? "text-danger"
                : "text-muted-foreground"
          }
        >
          {l.type === "add" ? "+" : l.type === "del" ? "-" : " "}
          {l.text}
        </div>
      ))}
      {diff.length > 80 && (
        <div className="text-faint-foreground">… {diff.length - 80} more lines</div>
      )}
    </pre>
  ) : (
    <pre className="m-0 whitespace-pre-wrap break-words p-2 font-code text-small leading-snug">
      {primary}
    </pre>
  );

  return (
    <div className="px-3 py-1.5">
      <DecisionCard
        data-testid={`decision-${req.id}`}
        tier={info.tier}
        tierLabel={info.label}
        tierReason={info.reason}
        intent={
          <span className="flex flex-wrap items-center gap-2">
            <Chip tone="accent" mono>
              {req.tool}
            </Chip>
            {diff ? primary : (secondary ?? "wants to run this")}
            {engine === "codex" && <Chip mono>codex</Chip>}
          </span>
        }
        context={session ? `${session.name} · ${req.cwd}` : req.cwd}
        preview={preview}
        deadline={req.ts + (req.timeoutMs ?? 90_000)}
        startedAt={req.ts}
        busy={busy}
        typedConfirm={info.tier === "high"}
        queuePosition={{ index, total }}
        actions={[
          {
            label: "Deny",
            variant: "danger",
            onClick: () => run(decideApproval(req, "deny", "user denied")),
          },
          {
            label: always ? "Cancel rule" : "Always…",
            variant: "secondary",
            onClick: () => setAlways((v) => !v),
          },
          {
            label: "Approve once",
            variant: "primary",
            onClick: () => run(decideApproval(req, "allow", "approved once")),
          },
        ]}
      />
      {always && (
        <div className="mx-1 rounded-b-3 border border-t-0 border-border bg-surface-1 px-3 py-2">
          <AlwaysPanel
            req={req}
            engine={engine}
            suggestions={suggestions}
            scope={scope}
            setScope={setScope}
            onCancel={() => setAlways(false)}
            onCommit={(s, effect) => run(commitAlways(req, engine, s, effect))}
          />
        </div>
      )}
    </div>
  );
}
