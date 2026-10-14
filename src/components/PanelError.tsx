import { TriangleAlert, X } from "lucide-react";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";

/// Shared error surface for workbench panels: the same message, Retry and
/// Dismiss affordances everywhere, announced as an alert.
export function PanelError({
  message,
  onRetry,
  onDismiss,
}: {
  message: string;
  onRetry?: () => void;
  onDismiss?: () => void;
}) {
  return (
    <div
      role="alert"
      className="m-2 flex items-start gap-2 rounded-2 border border-danger/40 bg-[var(--danger-08)] px-2.5 py-2 font-sans text-small text-foreground"
    >
      <TriangleAlert size={14} className="mt-0.5 shrink-0 text-danger" aria-hidden />
      <span className="min-w-0 flex-1 break-words">{message}</span>
      {onRetry && (
        <Button size="sm" variant="ghost" onClick={onRetry}>
          Retry
        </Button>
      )}
      {onDismiss && (
        <IconButton size="sm" label="Dismiss" icon={<X size={14} />} onClick={onDismiss} />
      )}
    </div>
  );
}
