import { useToastStore } from "../stores/toastStore";

export function Toasts() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  if (toasts.length === 0) return null;

  return (
    <div className="toast-stack" aria-live="polite" aria-atomic="true">
      {toasts.map((t) => (
        <button
          key={t.id}
          className={`toast toast-${t.tone}`}
          onClick={() => dismiss(t.id)}
          title="Dismiss"
        >
          {t.message}
        </button>
      ))}
    </div>
  );
}
