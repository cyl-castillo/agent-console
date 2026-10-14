import type { ReactElement, ReactNode } from "react";
import { Tooltip as BaseTooltip } from "@base-ui/react/tooltip";
import { cn } from "@/lib/cn";

/// Hover/focus hint. Replaces the 320 `title=` attributes: shows on focus
/// too, is readable, and never carries information the control lacks (the
/// control's own accessible name stays authoritative).
export function Tooltip({
  content,
  children,
  side = "bottom",
  className,
}: {
  content: ReactNode;
  /// The trigger element (a Button, IconButton, …).
  children: ReactElement;
  side?: "top" | "bottom" | "left" | "right";
  className?: string;
}) {
  return (
    <BaseTooltip.Root>
      <BaseTooltip.Trigger render={children} />
      <BaseTooltip.Portal>
        <BaseTooltip.Positioner side={side} sideOffset={6}>
          <BaseTooltip.Popup
            className={cn(
              "z-50 max-w-xs rounded-1 border border-border bg-surface-3 px-2 py-1 font-sans text-small text-foreground shadow-e2",
              "data-starting-style:opacity-0 data-ending-style:opacity-0 transition-opacity dur-1",
              className,
            )}
          >
            {content}
          </BaseTooltip.Popup>
        </BaseTooltip.Positioner>
      </BaseTooltip.Portal>
    </BaseTooltip.Root>
  );
}

export const TooltipProvider = BaseTooltip.Provider;
