import type { ReactNode } from "react";
import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { X } from "lucide-react";
import { cn } from "@/lib/cn";
import { IconButton } from "./IconButton";

/// Non-blocking side or bottom panel for context (a turn's inspector, the
/// "Always…" rules during an approval). Modal for focus, light on chrome.
export function Sheet({
  open,
  onOpenChange,
  title,
  side = "right",
  children,
  className,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  side?: "right" | "bottom";
  children: ReactNode;
  className?: string;
}) {
  const place =
    side === "right"
      ? "inset-y-0 right-0 w-[420px] max-w-[85vw] border-l data-starting-style:translate-x-4 data-ending-style:translate-x-4"
      : "inset-x-0 bottom-0 max-h-[70vh] border-t data-starting-style:translate-y-4 data-ending-style:translate-y-4";
  return (
    <BaseDialog.Root open={open} onOpenChange={onOpenChange}>
      <BaseDialog.Portal>
        <BaseDialog.Backdrop className="fixed inset-0 z-40 bg-black/30 data-starting-style:opacity-0 data-ending-style:opacity-0 transition-opacity dur-2" />
        <BaseDialog.Popup
          className={cn(
            "fixed z-50 flex flex-col overflow-hidden border-border bg-surface-1 font-sans text-body text-foreground shadow-e3",
            "data-starting-style:opacity-0 data-ending-style:opacity-0 transition-[opacity,transform] dur-2 motion-reduce:transition-none",
            place,
            className,
          )}
        >
          <div className="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3">
            <BaseDialog.Title className="m-0 flex-1 truncate text-small font-semibold uppercase tracking-wide text-faint-foreground">
              {title}
            </BaseDialog.Title>
            <BaseDialog.Close
              render={<IconButton label="Close" icon={<X size={16} />} size="sm" />}
            />
          </div>
          <div className="min-h-0 flex-1 overflow-auto p-3">{children}</div>
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}
