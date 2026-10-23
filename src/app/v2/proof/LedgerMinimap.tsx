import { useMemo } from "react";
import { cn } from "@/lib/cn";
import { useProofStore } from "../../../stores/proofStore";
import { useShellStore } from "../../../stores/shellStore";
import { useTerminalsStore } from "../../../stores/terminalsStore";
import type { ProofEvent } from "../../../types/domain";

const W = 1000;
const LANE_H = 18;
const PAD = 4;

const KIND_CLASS: Record<string, string> = {
  prompt: "fill-primary",
  turn_end: "fill-ok",
  approval_decision: "fill-warn",
  check_run: "fill-ok",
  verification: "fill-text-faint",
  commit: "fill-purple",
  rewind: "fill-warn",
  tool_result: "fill-text-faint",
  session_start: "fill-text-faint",
  model_switch: "fill-text-faint",
  context_injected: "fill-purple",
};

interface TurnSpan {
  turnId: string;
  lane: number;
  start: number;
  end: number;
  failed: boolean;
  open: boolean;
}

/// The ledger as a strip: one lane per session, a bar per turn, a tick per
/// event coloured by kind — approvals amber, finished turns green, failures
/// red, commits and injected memories purple. Click a bar to inspect the
/// turn on the right. The transcript below stays the record; this is the
/// map (the same shape Anthropic's Session Viewer settled on).
export function LedgerMinimap({ className }: { className?: string }) {
  const events = useProofStore((s) => s.events);
  const sessions = useTerminalsStore((s) => s.sessions);
  const selected = useShellStore((s) => s.selectedTurnId);
  const selectTurn = useShellStore((s) => s.selectTurn);

  const model = useMemo(() => {
    if (events.length === 0) return null;
    const laneIds: string[] = [];
    for (const e of events) {
      const id = e.termId ?? "other";
      if (!laneIds.includes(id)) laneIds.push(id);
    }
    const t0 = events[0].ts;
    const t1 = Math.max(events[events.length - 1].ts, t0 + 1);
    const x = (ts: number) => PAD + ((ts - t0) / (t1 - t0)) * (W - 2 * PAD);
    const spans = new Map<string, TurnSpan>();
    for (const e of events) {
      if (!e.turnId) continue;
      const lane = laneIds.indexOf(e.termId ?? "other");
      const sp = spans.get(e.turnId) ?? {
        turnId: e.turnId,
        lane,
        start: e.ts,
        end: e.ts,
        failed: false,
        open: true,
      };
      sp.end = Math.max(sp.end, e.ts);
      if (e.kind === "turn_end") {
        sp.open = false;
        sp.failed = (e.payload as { failed?: boolean }).failed === true;
      }
      spans.set(e.turnId, sp);
    }
    const laneName = (id: string) =>
      sessions.find((s) => s.id === id)?.name ?? (id === "other" ? "other" : id);
    return { laneIds, x, spans: [...spans.values()], laneName, events: events as ProofEvent[] };
  }, [events, sessions]);

  if (!model) return null;
  const height = model.laneIds.length * LANE_H + PAD * 2;

  return (
    <div
      className={cn("border-b border-border bg-surface-1 px-2 py-1", className)}
      data-testid="ledger-minimap"
    >
      <div className="flex">
        <div className="shrink-0 pr-2" style={{ paddingTop: PAD }}>
          {model.laneIds.map((id) => (
            <div
              key={id}
              className="truncate font-sans text-caption text-faint-foreground"
              style={{ height: LANE_H, lineHeight: `${LANE_H}px`, maxWidth: 120 }}
            >
              {model.laneName(id)}
            </div>
          ))}
        </div>
        <svg
          viewBox={`0 0 ${W} ${height}`}
          preserveAspectRatio="none"
          className="min-w-0 flex-1"
          style={{ height }}
          role="group"
          aria-label="Ledger timeline: one lane per session, one bar per turn"
        >
          {model.spans.map((sp) => {
            const y = PAD + sp.lane * LANE_H + 4;
            const x1 = model.x(sp.start);
            const x2 = Math.max(model.x(sp.end), x1 + 6);
            const isSel = sp.turnId === selected;
            return (
              <rect
                key={sp.turnId}
                x={x1}
                y={y}
                width={x2 - x1}
                height={LANE_H - 8}
                rx={2}
                className={cn(
                  "cursor-pointer",
                  sp.failed ? "fill-danger/40" : sp.open ? "fill-primary/30" : "fill-ok/30",
                  isSel ? "stroke-primary" : "stroke-transparent hover:stroke-text-dim",
                )}
                strokeWidth={2}
                vectorEffect="non-scaling-stroke"
                role="button"
                tabIndex={0}
                aria-label={`Turn ${sp.turnId}${sp.failed ? ", failed" : sp.open ? ", running" : ""}`}
                aria-pressed={isSel}
                onClick={() => selectTurn(isSel ? null : sp.turnId)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    selectTurn(isSel ? null : sp.turnId);
                  }
                }}
                data-testid={`minimap-turn-${sp.turnId}`}
              />
            );
          })}
          {model.events.map((e) => {
            const lane = model.laneIds.indexOf(e.termId ?? "other");
            const cy = PAD + lane * LANE_H + LANE_H / 2;
            const failed = e.kind === "turn_end" && (e.payload as { failed?: boolean }).failed;
            const cls = failed ? "fill-danger" : (KIND_CLASS[e.kind] ?? "fill-text-faint");
            return (
              <circle
                key={e.seq}
                cx={model.x(e.ts)}
                cy={cy}
                r={2.5}
                className={cn(cls, "pointer-events-none")}
                vectorEffect="non-scaling-stroke"
              />
            );
          })}
        </svg>
      </div>
    </div>
  );
}
