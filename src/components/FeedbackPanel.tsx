import { useEffect } from "react";

import { useFeedbackStore } from "../stores/feedbackStore";

export function FeedbackPanel() {
  const ctx = useFeedbackStore((s) => s.ctx);
  const title = useFeedbackStore((s) => s.title);
  const description = useFeedbackStore((s) => s.description);
  const category = useFeedbackStore((s) => s.category);
  const severity = useFeedbackStore((s) => s.severity);
  const status = useFeedbackStore((s) => s.status);
  const error = useFeedbackStore((s) => s.error);
  const lastUrl = useFeedbackStore((s) => s.lastUrl);
  const setField = useFeedbackStore((s) => s.setField);
  const submit = useFeedbackStore((s) => s.submit);
  const refreshContext = useFeedbackStore((s) => s.refreshContext);
  const reset = useFeedbackStore((s) => s.reset);

  useEffect(() => {
    void refreshContext();
  }, [refreshContext]);

  const disabled = status === "submitting";

  return (
    <div className="feedback-panel">
      <div className="feedback-header">
        <h3>Feedback</h3>
        <p className="feedback-help">
          Crea un issue en <code>cyl-castillo/agent-console</code> con el contexto adjunto. Only
          visible to the dev team (<code>AGENT_CONSOLE_DEV=1</code>).
        </p>
      </div>

      <label className="feedback-label">
        Title
        <input
          className="feedback-input"
          value={title}
          onChange={(e) => setField({ title: e.target.value })}
          placeholder="Short summary (one line)"
          disabled={disabled}
        />
      </label>

      <div className="feedback-row">
        <label className="feedback-label">
          Category
          <select
            className="feedback-input"
            value={category}
            onChange={(e) => setField({ category: e.target.value as typeof category })}
            disabled={disabled}
          >
            <option value="bug">Bug</option>
            <option value="feature">Feature</option>
            <option value="ux">UX</option>
            <option value="other">Otro</option>
          </select>
        </label>
        <label className="feedback-label">
          Severidad
          <select
            className="feedback-input"
            value={severity}
            onChange={(e) => setField({ severity: e.target.value as typeof severity })}
            disabled={disabled}
          >
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
          </select>
        </label>
      </div>

      <label className="feedback-label">
        Description
        <textarea
          className="feedback-input feedback-textarea"
          value={description}
          onChange={(e) => setField({ description: e.target.value })}
          placeholder="What happens, what you expected, repro steps if it's a bug…"
          rows={8}
          disabled={disabled}
        />
      </label>

      {ctx && (
        <div className="feedback-context">
          <div className="feedback-context-title">Will be attached:</div>
          <ul>
            <li>
              App: <code>v{ctx.appVersion}</code>
            </li>
            <li>
              OS: <code>{ctx.os}</code>
            </li>
            {ctx.projectName && (
              <li>
                Project: <code>{ctx.projectName}</code>
              </li>
            )}
            {ctx.branch && (
              <li>
                Branch: <code>{ctx.branch}</code>
              </li>
            )}
          </ul>
        </div>
      )}

      <div className="feedback-actions">
        <button
          className="feedback-submit"
          onClick={() => void submit()}
          disabled={disabled || !title.trim() || !description.trim()}
        >
          {status === "submitting" ? "Submitting…" : "Submit feedback"}
        </button>
        <button className="feedback-reset" onClick={reset} disabled={disabled}>
          Clear
        </button>
      </div>

      {status === "success" && lastUrl && (
        <div className="feedback-success">
          ✔ Issue creado:{" "}
          <a href={lastUrl} target="_blank" rel="noreferrer">
            {lastUrl}
          </a>
        </div>
      )}
      {status === "error" && error && <div className="feedback-error">{error}</div>}
    </div>
  );
}
