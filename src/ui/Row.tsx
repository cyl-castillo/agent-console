import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { cn } from "@/lib/cn";

export interface RowProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "title"> {
  leading?: ReactNode;
  title: ReactNode;
  /// Secondary line (summary, path, age). Truncates.
  subtitle?: ReactNode;
  trailing?: ReactNode;
  selected?: boolean;
}

/// Dense list row (28 px, 36 px with subtitle): leading glyph, title,
/// optional subtitle, trailing meta. A button, so keyboard and agents can
/// activate it; selection is announced, not only coloured.
export const Row = forwardRef<HTMLButtonElement, RowProps>(function Row(
  { leading, title, subtitle, trailing, selected, className, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-2 px-3 text-left font-sans text-body text-foreground transition-colors dur-1",
        subtitle ? "min-h-9 py-1" : "h-7",
        "hover:bg-[var(--hover)] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
        selected && "bg-[var(--accent-12)]",
        className,
      )}
      {...rest}
    >
      {leading && <span className="flex w-4 shrink-0 items-center justify-center">{leading}</span>}
      <span className="min-w-0 flex-1">
        <span className="block truncate">{title}</span>
        {subtitle && (
          <span className="block truncate text-small text-muted-foreground">{subtitle}</span>
        )}
      </span>
      {trailing && (
        <span className="shrink-0 font-code text-caption text-muted-foreground tabular-nums">
          {trailing}
        </span>
      )}
    </button>
  );
});
