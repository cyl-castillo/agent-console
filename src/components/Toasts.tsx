import { useToastStore } from "../stores/toastStore";
import { reportProblem } from "../lib/reportProblem";

export function Toasts() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  if (toasts.length === 0) return null;

  const glyph = { success: "✓", error: "⚠", info: "›" } as const;

  return (
    <div className="toast-stack" aria-live="polite" aria-atomic="true">
      {toasts.map((t) => (
        // A div (not a button) so error toasts can host a nested Report action.
        <div
          key={t.id}
          className={`toast toast-${t.tone}`}
          onClick={() => dismiss(t.id)}
          role="status"
          title="Dismiss"
        >
          <span className="toast-glyph" aria-hidden="true">{glyph[t.tone]}</span>
          <span className="toast-msg">{t.message}</span>
          {t.tone === "error" && (
            // A failure is the moment a field report is worth the most — one
            // click opens a GitHub issue with the exact error prefilled.
            <button
              className="toast-report"
              onClick={(e) => { e.stopPropagation(); void reportProblem(t.message); }}
              title="Report this problem on GitHub (opens a prefilled issue)"
            >Report</button>
          )}
          {t.tone === "error" && <span className="toast-dismiss" aria-hidden="true">✕</span>}
        </div>
      ))}
    </div>
  );
}
