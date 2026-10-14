import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { cn } from "@/lib/cn";

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /// Required: icon-only controls have no visible text, so the name IS the
  /// accessible label (and the tooltip, when wrapped in one).
  label: string;
  icon: ReactNode;
  size?: "sm" | "md";
  active?: boolean;
}

/// 24 or 28 px square hit target (WCAG 2.5.8) around a 16 px icon.
export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, icon, size = "md", active, className, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      aria-label={label}
      aria-pressed={active}
      className={cn(
        "inline-flex items-center justify-center rounded-1 text-muted-foreground transition-colors dur-2",
        "hover:bg-[var(--hover)] hover:text-foreground",
        "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring",
        "disabled:pointer-events-none disabled:opacity-50",
        size === "sm" ? "size-6" : "size-7",
        active && "bg-[var(--accent-12)] text-primary",
        className,
      )}
      {...rest}
    >
      <span aria-hidden className="inline-flex shrink-0 [&>svg]:size-4">
        {icon}
      </span>
    </button>
  );
});
