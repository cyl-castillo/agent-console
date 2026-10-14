import { useProofStore } from "../../../stores/proofStore";
import { useShellStore } from "../../../stores/shellStore";
import { Chip } from "../../../ui/Chip";
import { EmptyState } from "../../../ui/EmptyState";
import { ShieldCheck } from "lucide-react";

const fmt = (ts: number) =>
  new Date(ts).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });

/// The selected turn's events, as the ledger holds them: sequence, time,
/// kind, actor, the hash link, and the payload keys. Enough to see what was
/// recorded without leaving the map; the Ledger view below has the export.
export function TurnInspector() {
  const turnId = useShellStore((s) => s.selectedTurnId);
  const events = useProofStore((s) => s.events);
  const report = useProofStore((s) => s.report);
  const mine = turnId ? events.filter((e) => e.turnId === turnId) : [];
  if (!turnId || mine.length === 0) {
    return (
      <EmptyState
        icon={<ShieldCheck />}
        title="No turn selected"
        description="Click a bar in the ledger map to see exactly what was witnessed for that turn."
      />
    );
  }
  const first = mine[0];
  const last = mine[mine.length - 1];
  return (
    <div className="flex flex-col gap-2 p-3 font-sans text-body" data-testid="turn-inspector">
      <div className="flex flex-wrap items-center gap-1.5">
        <Chip mono>{turnId}</Chip>
        <Chip tone={report?.ok ? "ok" : report ? "danger" : "neutral"}>
          {report
            ? report.ok
              ? "chain verified"
              : `chain broken at ${report.brokenAtSeq ?? "?"}`
            : "unverified"}
        </Chip>
        <span className="font-code text-caption text-faint-foreground tabular-nums">
          seq {first.seq}–{last.seq} · {mine.length} events
        </span>
      </div>
      <ol className="m-0 flex list-none flex-col gap-1 p-0">
        {mine.map((e) => {
          const keys = Object.keys(e.payload ?? {});
          return (
            <li key={e.seq} className="rounded-2 border border-border bg-surface-1 px-2 py-1.5">
              <div className="flex items-center gap-2 text-caption text-muted-foreground">
                <span className="font-code tabular-nums">#{e.seq}</span>
                <span className="font-code tabular-nums">{fmt(e.ts)}</span>
                <Chip
                  tone={
                    e.kind === "approval_decision"
                      ? "warn"
                      : e.kind === "turn_end"
                        ? "ok"
                        : "neutral"
                  }
                  mono
                >
                  {e.kind}
                </Chip>
                <span>{e.actor}</span>
                <span className="ml-auto font-code">{e.hash.slice(0, 8)}…</span>
              </div>
              {keys.length > 0 && (
                <div className="mt-1 truncate font-code text-caption text-foreground">
                  {keys
                    .map((k) => {
                      const v = (e.payload as Record<string, unknown>)[k];
                      const text =
                        typeof v === "string"
                          ? v
                          : Array.isArray(v)
                            ? `[${v.length}]`
                            : typeof v === "object" && v
                              ? "{…}"
                              : String(v);
                      return `${k}: ${text.length > 60 ? `${text.slice(0, 60)}…` : text}`;
                    })
                    .join(" · ")}
                </div>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}
