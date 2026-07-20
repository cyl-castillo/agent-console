/// Shared error surface for workbench panels. Before this, each panel rendered
/// fetch errors differently — some dead-end inline red text, some dismissible,
/// only a couple with a Retry. This gives every panel the same affordances.
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
    <div className="panel-error" role="alert">
      <span className="panel-error-icon" aria-hidden="true">
        ⚠
      </span>
      <span className="panel-error-msg">{message}</span>
      {onRetry && (
        <button className="btn btn-link btn-sm" onClick={onRetry}>
          Retry
        </button>
      )}
      {onDismiss && (
        <button
          className="btn btn-link btn-sm panel-error-dismiss"
          onClick={onDismiss}
          aria-label="Dismiss"
          title="Dismiss"
        >
          ✕
        </button>
      )}
    </div>
  );
}
