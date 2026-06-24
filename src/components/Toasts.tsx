import { useToastStore } from "../stores/toastStore";

export function Toasts() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  if (toasts.length === 0) return null;

  const glyph = { success: "✓", error: "⚠", info: "›" } as const;

  return (
    <div className="toast-stack" aria-live="polite" aria-atomic="true">
      {toasts.map((t) => (
        <button
          key={t.id}
          className={`toast toast-${t.tone}`}
          onClick={() => dismiss(t.id)}
          title="Dismiss"
        >
          <span className="toast-glyph" aria-hidden="true">{glyph[t.tone]}</span>
          <span className="toast-msg">{t.message}</span>
          {t.tone === "error" && <span className="toast-dismiss" aria-hidden="true">✕</span>}
        </button>
      ))}
    </div>
  );
}
