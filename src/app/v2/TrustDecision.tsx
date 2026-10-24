import { useState } from "react";
import { surfaceSummary, trustTier } from "../../lib/trust";
import { useTrustStore } from "../../stores/trustStore";
import { useToastStore } from "../../stores/toastStore";
import type { TrustStatus } from "../../types/domain";
import { Chip } from "../../ui/Chip";
import { DecisionCard } from "../../ui/DecisionCard";

/// F4: trust the folder before an agent starts in it. The card lists what
/// the folder would inject into the agents — hooks, setup commands,
/// plugins, MCP servers, rules, skills, instructions — with each file's
/// hash, so the decision is about something concrete. Both answers are
/// witnessed as `trust_decision`; a changed surface asks again.
export function TrustDecision({
  status,
  projectName,
}: {
  status: TrustStatus;
  projectName: string;
}) {
  const decide = useTrustStore((s) => s.decide);
  const showToast = useToastStore((s) => s.show);
  const [busy, setBusy] = useState(false);
  const [showFiles, setShowFiles] = useState(false);
  const tier = trustTier(status);
  const changed = status.state === "changed";
  const answer = async (decision: "trusted" | "declined") => {
    setBusy(true);
    try {
      await decide(decision);
    } catch (e) {
      showToast(`Couldn't record the decision: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };
  const executing = status.surface.files.filter((f) =>
    ["hooks", "setup", "plugins", "mcp"].includes(f.category),
  );
  return (
    <div className="px-3 py-1.5">
      <DecisionCard
        data-testid="trust-decision"
        tier={tier}
        tierLabel={changed ? "Folder changed since you trusted it" : "Trust this folder?"}
        tierReason={
          tier === "high"
            ? "it carries files the agents execute the moment they start"
            : "only instructions and skills; nothing runs by itself"
        }
        typedConfirm={false}
        busy={busy}
        intent={
          <span className="flex flex-wrap items-center gap-2">
            <Chip mono>{projectName}</Chip>
            {surfaceSummary(status)}
          </span>
        }
        context={`${status.surface.files.length} file${status.surface.files.length === 1 ? "" : "s"} · digest ${status.surface.digest.slice(0, 12)}… · agents wait in their terminals until you answer · recorded in the ledger`}
        preview={
          <div className="flex flex-col gap-1 px-2 py-1.5 font-sans text-small text-foreground">
            {executing.length > 0 && (
              <ul className="m-0 list-none p-0 font-code text-caption">
                {executing.map((f) => (
                  <li key={f.path}>
                    <Chip tone="danger" mono>
                      {f.category}
                    </Chip>{" "}
                    {f.path} <span className="text-faint-foreground">{f.sha256.slice(0, 10)}</span>
                  </li>
                ))}
              </ul>
            )}
            {status.surface.files.length > executing.length && (
              <button
                type="button"
                className="self-start text-caption text-muted-foreground underline"
                onClick={() => setShowFiles((v) => !v)}
              >
                {showFiles ? "hide" : "show"} the other{" "}
                {status.surface.files.length - executing.length} files
              </button>
            )}
            {showFiles && (
              <ul className="m-0 list-none p-0 font-code text-caption text-muted-foreground">
                {status.surface.files
                  .filter((f) => !executing.includes(f))
                  .map((f) => (
                    <li key={f.path}>
                      {f.category} · {f.path}{" "}
                      <span className="text-faint-foreground">{f.sha256.slice(0, 10)}</span>
                    </li>
                  ))}
              </ul>
            )}
          </div>
        }
        actions={[
          { label: "Don't trust", variant: "danger", onClick: () => void answer("declined") },
          { label: "Trust this folder", variant: "primary", onClick: () => void answer("trusted") },
        ]}
      />
    </div>
  );
}
