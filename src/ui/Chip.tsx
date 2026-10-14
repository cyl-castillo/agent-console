import type { HTMLAttributes, ReactNode } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/cn";

export const chipVariants = cva(
  "inline-flex items-center gap-1 rounded-pill px-1.5 h-[18px] text-caption font-sans leading-none whitespace-nowrap",
  {
    variants: {
      tone: {
        neutral: "bg-surface-2 text-muted-foreground border border-border",
        accent: "bg-[var(--accent-12)] text-[var(--accent-ink)]",
        ok: "bg-[var(--ok-12)] text-[var(--ok-ink)]",
        warn: "bg-[var(--warn-12)] text-[var(--warn-ink)]",
        danger: "bg-[var(--danger-12)] text-[var(--danger-ink)]",
        purple: "bg-[var(--purple-12)] text-[var(--purple-ink)]",
      },
    },
    defaultVariants: { tone: "neutral" },
  },
);

export interface ChipProps
  extends HTMLAttributes<HTMLSpanElement>, VariantProps<typeof chipVariants> {
  icon?: ReactNode;
  /// Monospace content (ids, engines, counts).
  mono?: boolean;
}

/// Small labelled status: an engine, a count, a state. Never interactive —
/// use Button for anything clickable.
export function Chip({ tone, icon, mono, className, children, ...rest }: ChipProps) {
  return (
    <span className={cn(chipVariants({ tone }), mono && "font-code", className)} {...rest}>
      {icon}
      {children}
    </span>
  );
}

/// A count bubble on a tab or button.
export function Badge({ count, className }: { count: number; className?: string }) {
  if (count <= 0) return null;
  return (
    <span
      className={cn(
        "inline-flex h-4 min-w-4 items-center justify-center rounded-pill bg-primary px-1 font-code text-[10px] leading-none text-background tabular-nums",
        className,
      )}
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}
