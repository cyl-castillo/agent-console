import { useEffect, useState } from "react";
import { parseHosts } from "../../lib/perimeter";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { Chip } from "../../ui/Chip";
import { Input } from "../../ui/Field";
import { Tooltip } from "../../ui/Tooltip";

/// D14 in the session popover: the agent's own id, where it runs, and the
/// perimeter the approval gate enforces (empty = none declared: nothing
/// changes). Hosts are exact or `.suffix`.
export function IdentityRow({ session }: { session: TerminalSession }) {
  const setAllowedHosts = useTerminalsStore((s) => s.setAllowedHosts);
  const [draft, setDraft] = useState((session.allowedHosts ?? []).join(", "));
  useEffect(
    () => setDraft((session.allowedHosts ?? []).join(", ")),
    [session.id, session.allowedHosts],
  );
  const short = session.agentId?.replace("urn:agent-console:agent:", "").slice(0, 8);
  return (
    <div className="mt-1 flex flex-col gap-1 text-caption" data-testid="identity-row">
      <div className="flex items-center gap-1.5">
        <Tooltip content={session.agentId ?? "no agent id yet (spawn the session)"}>
          <span>
            <Chip tone="purple" mono>
              agent:{short ?? "—"}
            </Chip>
          </span>
        </Tooltip>
        <Chip mono>{session.runsIn ?? "local"}</Chip>
        <span className="text-faint-foreground">acts for you, under its own id</span>
      </div>
      <label className="flex items-center gap-1 text-faint-foreground">
        hosts
        <Input
          aria-label="Allowed hosts for this session (comma separated; empty = no perimeter)"
          className="h-5 flex-1 px-1 text-caption"
          placeholder="none declared"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => setAllowedHosts(session.id, parseHosts(draft))}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          }}
        />
      </label>
    </div>
  );
}
