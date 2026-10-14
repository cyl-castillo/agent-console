import { useRef, type ReactNode } from "react";
import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { X } from "lucide-react";
import { cn } from "@/lib/cn";
import { IconButton } from "./IconButton";

/// Modal dialog: focus trapped inside, restored to the opener on close,
/// Escape closes, background inert. Decisions only — contextual panels use
/// Sheet or Popover.
export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  size = "md",
  className,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  size?: "sm" | "md" | "lg";
  className?: string;
}) {
  const width = { sm: "w-[360px]", md: "w-[480px]", lg: "w-[640px]" }[size];
  const body = useRef<HTMLDivElement>(null);
  // Focus lands on the first field or an explicit [data-autofocus] target;
  // otherwise on the body wrapper — never on the close button, so Enter
  // never dismisses by accident and the ring does not open on an "×".
  const initialFocus = () =>
    (body.current?.querySelector<HTMLElement>("[data-autofocus], input, textarea, select") ??
      body.current) as HTMLElement | null;
  return (
    <BaseDialog.Root open={open} onOpenChange={onOpenChange}>
      <BaseDialog.Portal>
        <BaseDialog.Backdrop className="fixed inset-0 z-40 bg-black/50 data-starting-style:opacity-0 data-ending-style:opacity-0 transition-opacity dur-2" />
        <BaseDialog.Popup
          initialFocus={initialFocus}
          className={cn(
            "fixed left-1/2 top-1/2 z-50 max-h-[85vh] max-w-[calc(100vw-32px)] -translate-x-1/2 -translate-y-1/2 overflow-auto",
            "rounded-3 border border-border bg-surface-2 p-4 font-sans text-body text-foreground shadow-e3",
            "data-starting-style:scale-[0.98] data-starting-style:opacity-0 data-ending-style:opacity-0 transition-[opacity,transform] dur-2 motion-reduce:transition-none",
            width,
            className,
          )}
        >
          <div className="mb-3 flex items-start gap-2">
            <div className="min-w-0 flex-1">
              <BaseDialog.Title className="m-0 text-title font-semibold">{title}</BaseDialog.Title>
              {description && (
                <BaseDialog.Description className="mt-1 text-body text-muted-foreground">
                  {description}
                </BaseDialog.Description>
              )}
            </div>
            <BaseDialog.Close
              render={<IconButton label="Close" icon={<X size={16} />} size="sm" />}
            />
          </div>
          <div ref={body} tabIndex={-1} className="outline-none">
            {children}
          </div>
          {footer && <div className="mt-4 flex justify-end gap-2">{footer}</div>}
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}
