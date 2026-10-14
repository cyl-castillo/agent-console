import { useEffect, useState, type ReactNode } from "react";
import { cn } from "@/lib/cn";
import { Button } from "./Button";
import { Chip } from "./Chip";
import { Kbd } from "./Kbd";
import { Progress } from "./Progress";
import { Input } from "./Field";
import { CONFIRM_WORD, type DecisionTier } from "../lib/decisions";

export interface DecisionAction {
  label: string;
  variant?: "primary" | "secondary" | "ghost" | "danger";
  onClick: () => void;
  /// Keyboard hint rendered as key caps.
  keys?: string[];
  disabled?: boolean;
}

const TIER_TONE: Record<DecisionTier, "ok" | "warn" | "danger"> = {
  low: "ok",
  medium: "warn",
  high: "danger",
};

/// A decision as a card (D4): what the agent wants (the intent), the preview
/// you decide on, its tier, how long it will wait, and the actions. The
/// countdown is a bar, not a number that pushes. A high tier asks for a typed
/// word before the primary action enables.
export function DecisionCard({
  tier,
  tierLabel,
  tierReason,
  intent,
  context,
  preview,
  deadline,
  startedAt,
  actions,
  busy,
  typedConfirm,
  queuePosition,
  className,
  "data-testid": testId,
}: {
  tier: DecisionTier;
  tierLabel: string;
  tierReason?: string;
  intent: ReactNode;
  context?: ReactNode;
  preview?: ReactNode;
  /// Epoch ms when the request expires; with startedAt it draws the bar.
  deadline?: number;
  startedAt?: number;
  actions: DecisionAction[];
  busy?: boolean;
  /// High tier: the primary action stays disabled until this word is typed.
  typedConfirm?: boolean;
  queuePosition?: { index: number; total: number };
  className?: string;
  "data-testid"?: string;
}) {
  const [typed, setTyped] = useState("");
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!deadline) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [deadline]);
  const confirmed = !typedConfirm || typed.trim().toLowerCase() === CONFIRM_WORD;
  const total = deadline && startedAt ? deadline - startedAt : 0;
  const left = deadline ? Math.max(0, deadline - now) : 0;
  const pct = total > 0 ? Math.round((left / total) * 100) : null;
  const expired = deadline !== undefined && left === 0;

  return (
    <article
      className={cn(
        "rounded-3 border bg-surface-1 font-sans text-body text-foreground",
        tier === "high"
          ? "border-danger/50"
          : tier === "medium"
            ? "border-warn/40"
            : "border-border",
        className,
      )}
      aria-busy={busy || undefined}
      data-testid={testId}
      data-tier={tier}
    >
      <header className="flex items-center gap-2 border-b border-border px-3 py-2">
        <Chip tone={TIER_TONE[tier]}>{tierLabel}</Chip>
        {tierReason && (
          <span className="truncate text-small text-muted-foreground">{tierReason}</span>
        )}
        <span className="ml-auto" />
        {queuePosition && queuePosition.total > 1 && (
          <span className="font-code text-caption text-faint-foreground tabular-nums">
            {queuePosition.index + 1} of {queuePosition.total}
          </span>
        )}
      </header>
      <div className="flex flex-col gap-2 px-3 py-2">
        <div className="text-lead font-medium">{intent}</div>
        {context && <div className="text-small text-muted-foreground">{context}</div>}
        {preview && (
          <div className="overflow-auto rounded-2 border border-border bg-background">
            {preview}
          </div>
        )}
        {deadline && (
          <div className="flex items-center gap-2">
            <Progress
              value={pct}
              label={
                expired ? "Expired" : `${Math.ceil(left / 1000)} seconds before the agent gives up`
              }
              tone={expired ? "danger" : left < 15_000 ? "warn" : "accent"}
              className="flex-1"
            />
            <span className="font-code text-caption text-faint-foreground tabular-nums">
              {expired ? "expired" : `${Math.ceil(left / 1000)}s`}
            </span>
          </div>
        )}
        {typedConfirm && (
          <label className="flex items-center gap-2 text-small text-muted-foreground">
            <span>
              Type <span className="font-code text-foreground">{CONFIRM_WORD}</span> to enable
            </span>
            <Input
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              className="h-6 w-32"
              aria-label={`Type ${CONFIRM_WORD} to enable approving`}
              autoComplete="off"
            />
          </label>
        )}
      </div>
      <footer className="flex flex-wrap items-center justify-end gap-2 border-t border-border px-3 py-2">
        {actions.map((a, i) => {
          const primary = a.variant === "primary";
          return (
            <Button
              key={a.label}
              variant={a.variant ?? (i === actions.length - 1 ? "primary" : "secondary")}
              disabled={busy || a.disabled || (primary && !confirmed)}
              onClick={a.onClick}
              trailing={a.keys?.map((k) => (
                <Kbd key={k}>{k}</Kbd>
              ))}
            >
              {a.label}
            </Button>
          );
        })}
      </footer>
    </article>
  );
}
