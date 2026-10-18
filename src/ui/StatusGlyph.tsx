import { cn } from "@/lib/cn";

export type AgentState =
  "needs-you" | "working" | "review" | "done" | "idle" | "failed" | "stopped";

const META: Record<AgentState, { label: string; className: string; glyph: string }> = {
  "needs-you": { label: "Needs you", className: "text-warn", glyph: "●" },
  working: { label: "Working", className: "text-primary", glyph: "✻" },
  review: { label: "Ready for review", className: "text-purple", glyph: "◆" },
  done: { label: "Done", className: "text-ok", glyph: "✓" },
  idle: { label: "Idle", className: "text-muted-foreground", glyph: "∙" },
  failed: { label: "Failed", className: "text-danger", glyph: "✕" },
  stopped: { label: "Stopped", className: "text-faint-foreground", glyph: "○" },
};

/// The attention states, each with a glyph AND a name — never colour alone.
/// `working` pulses unless the user prefers reduced motion. `done` is a
/// finished turn; `idle` is a live session with nothing happening.
export function StatusGlyph({
  state,
  showLabel,
  className,
}: {
  state: AgentState;
  showLabel?: boolean;
  className?: string;
}) {
  const m = META[state];
  return (
    <span
      role="img"
      aria-label={m.label}
      title={showLabel ? undefined : m.label}
      className={cn(
        "inline-flex items-center gap-1.5 font-sans text-small",
        m.className,
        className,
      )}
    >
      <span
        aria-hidden
        className={cn(
          "inline-block w-3 text-center leading-none",
          state === "working" && "animate-pulse motion-reduce:animate-none",
        )}
      >
        {m.glyph}
      </span>
      {showLabel && <span className="text-foreground">{m.label}</span>}
    </span>
  );
}

export const agentStateLabel = (s: AgentState) => META[s].label;
