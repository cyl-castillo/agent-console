import { Progress as BaseProgress } from "@base-ui/react/progress";
import { cn } from "@/lib/cn";

/// Linear progress, determinate (0–100) or indeterminate (value null).
export function Progress({
  value,
  label,
  tone = "accent",
  className,
}: {
  value: number | null;
  label: string;
  tone?: "accent" | "ok" | "warn" | "danger";
  className?: string;
}) {
  const color = { accent: "bg-primary", ok: "bg-ok", warn: "bg-warn", danger: "bg-danger" }[tone];
  return (
    <BaseProgress.Root value={value} aria-label={label} className={cn("block w-full", className)}>
      <BaseProgress.Track className="relative block h-1 w-full overflow-hidden rounded-pill bg-border">
        <BaseProgress.Indicator
          className={cn(
            "block h-full rounded-pill transition-[width] dur-3",
            color,
            value === null &&
              "w-1/3 animate-[progress-slide_1.2s_ease-in-out_infinite] motion-reduce:animate-none",
          )}
        />
      </BaseProgress.Track>
    </BaseProgress.Root>
  );
}
