import { cn } from "@/lib/cn";

/// Spend against a cap as a ring: accent while under 80 %, amber to 100 %,
/// red past it. Always paired with the numbers — the ring is a glance, the
/// text is the fact.
export function BudgetRing({
  spent,
  cap,
  size = 18,
  className,
}: {
  spent: number;
  cap: number;
  size?: number;
  className?: string;
}) {
  const pct = cap > 0 ? Math.min(1, spent / cap) : 0;
  const r = (size - 3) / 2;
  const c = 2 * Math.PI * r;
  const tone =
    cap <= 0
      ? "text-faint-foreground"
      : pct >= 1
        ? "text-danger"
        : pct >= 0.8
          ? "text-warn"
          : "text-primary";
  const label =
    cap > 0 ? `$${spent.toFixed(2)} of $${cap.toFixed(2)}` : `$${spent.toFixed(2)}, no cap`;
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 font-code text-caption tabular-nums",
        className,
      )}
    >
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        role="img"
        aria-label={label}
        className={tone}
      >
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke="currentColor"
          strokeOpacity={0.2}
          strokeWidth={2}
        />
        {cap > 0 && (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
            strokeLinecap="round"
            strokeDasharray={c}
            strokeDashoffset={c * (1 - pct)}
            transform={`rotate(-90 ${size / 2} ${size / 2})`}
          />
        )}
      </svg>
      <span aria-hidden className={tone}>
        {label}
      </span>
    </span>
  );
}
