import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

/// Uppercase section label with an optional count and trailing actions —
/// the one pattern the ~40 ad-hoc section titles collapse into.
export function SectionHeader({
  title,
  count,
  actions,
  className,
  as: Tag = "h3",
}: {
  title: string;
  count?: number;
  actions?: ReactNode;
  className?: string;
  as?: "h2" | "h3" | "h4";
}) {
  return (
    <div className={cn("flex h-7 items-center gap-2 px-3", className)}>
      <Tag className="m-0 font-sans text-caption font-semibold uppercase tracking-wide text-faint-foreground">
        {title}
      </Tag>
      {typeof count === "number" && (
        <span className="font-code text-caption text-faint-foreground tabular-nums">{count}</span>
      )}
      {actions && <span className="ml-auto inline-flex items-center gap-1">{actions}</span>}
    </div>
  );
}
