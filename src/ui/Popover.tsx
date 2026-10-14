import type { ReactElement, ReactNode } from "react";
import { Popover as BasePopover } from "@base-ui/react/popover";
import { cn } from "@/lib/cn";

/// Anchored, dismissable panel for contextual controls (filters, session
/// details). Focus moves in and returns; Escape and outside click close.
export function Popover({
  trigger,
  title,
  children,
  side = "bottom",
  align = "start",
  open,
  onOpenChange,
  className,
}: {
  trigger: ReactElement;
  title?: string;
  children: ReactNode;
  side?: "top" | "bottom" | "left" | "right";
  align?: "start" | "center" | "end";
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  className?: string;
}) {
  return (
    <BasePopover.Root open={open} onOpenChange={onOpenChange}>
      <BasePopover.Trigger render={trigger} />
      <BasePopover.Portal>
        <BasePopover.Positioner side={side} align={align} sideOffset={6} className="z-50">
          <BasePopover.Popup
            className={cn(
              "min-w-56 rounded-3 border border-border bg-surface-3 p-3 font-sans text-body text-foreground shadow-e2",
              "data-starting-style:opacity-0 data-ending-style:opacity-0 transition-opacity dur-1",
              className,
            )}
          >
            {title && (
              <BasePopover.Title className="mb-2 text-small font-semibold text-muted-foreground">
                {title}
              </BasePopover.Title>
            )}
            {children}
          </BasePopover.Popup>
        </BasePopover.Positioner>
      </BasePopover.Portal>
    </BasePopover.Root>
  );
}
