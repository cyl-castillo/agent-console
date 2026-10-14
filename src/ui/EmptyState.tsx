import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

/// An empty surface that teaches: icon, one title, one line of copy, one
/// action, optional checklist. Never a lone spinner, never a blank panel.
export function EmptyState({
  icon,
  title,
  description,
  action,
  checklist,
  className,
}: {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  checklist?: { label: string; done: boolean }[];
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 px-6 py-8 text-center font-sans",
        className,
      )}
    >
      {icon && <span className="text-faint-foreground [&>svg]:size-6">{icon}</span>}
      <h3 className="m-0 text-lead font-semibold text-foreground">{title}</h3>
      {description && <p className="m-0 max-w-sm text-body text-muted-foreground">{description}</p>}
      {action && <div className="mt-1">{action}</div>}
      {checklist && (
        <ul className="mt-2 flex list-none flex-col gap-1 p-0 text-left text-body">
          {checklist.map((c) => (
            <li key={c.label} className="flex items-center gap-2">
              <span
                aria-hidden
                className={cn(
                  "inline-flex size-4 items-center justify-center rounded-pill border text-[10px]",
                  c.done ? "border-ok bg-[var(--ok-12)] text-ok" : "border-border text-transparent",
                )}
              >
                ✓
              </span>
              <span
                className={cn(c.done ? "text-faint-foreground line-through" : "text-foreground")}
              >
                {c.label}
              </span>
              <span className="sr-only">{c.done ? "(done)" : "(pending)"}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
