import { useEffect, useState } from "react";

import { useAdvisorStore, type AdvisorItem } from "../stores/advisorStore";

export function AdvisorPanel() {
  const status = useAdvisorStore((s) => s.status);
  const items = useAdvisorStore((s) => s.items);
  const errorMessage = useAdvisorStore((s) => s.errorMessage);
  const analyze = useAdvisorStore((s) => s.analyze);
  const reset = useAdvisorStore((s) => s.reset);

  // Elapsed-time counter while analyzing, so the wait reads as live progress
  // rather than a frozen panel.
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (status !== "analyzing") { setElapsed(0); return; }
    setElapsed(0);
    const started = Date.now();
    const t = setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => clearInterval(t);
  }, [status]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">advisor</span>
        <span className="spacer" />
        {status === "results" && (
          <button className="workbench-action" onClick={reset} title="Clear results">×</button>
        )}
        <button
          className="workbench-action"
          onClick={analyze}
          disabled={status === "analyzing"}
          title="Analyze project and recommend skills"
        >
          {status === "analyzing" ? "…" : "↻"}
        </button>
      </div>

      <div className="workbench-body">
        {status === "idle" && (
          <section className="wb-section">
            <p className="wb-hint">
              The Advisor opens a background Claude session in this project,
              analyzes its structure, and proposes skills you can create with
              one click. Nothing is written to disk until you confirm.
            </p>
            <button className="wb-cta" onClick={analyze}>Analyze project</button>
          </section>
        )}

        {status === "analyzing" && (
          <section className="wb-section">
            <div className="wb-working">
              <span className="wb-spinner" />
              <div className="wb-working-text">
                <div className="wb-working-title">
                  Working… <span className="wb-working-elapsed">{formatElapsed(elapsed)}</span>
                </div>
                <div className="wb-working-sub">
                  Analyzing the project with Claude in the background (usually
                  30–90s). You can keep using the app — results land here when ready.
                </div>
              </div>
            </div>
          </section>
        )}

        {status === "error" && (
          <section className="wb-section">
            <div className="wb-section-title">analysis failed</div>
            <p className="wb-hint" style={{ whiteSpace: "pre-wrap" }}>
              {errorMessage}
            </p>
            <button className="wb-cta" onClick={analyze}>Retry</button>
          </section>
        )}

        {status === "results" && (
          <section className="wb-section">
            <div className="wb-section-title">
              recommendations
              <span className="wb-count">{items.length}</span>
            </div>
            {items.length === 0 ? (
              <p className="wb-hint">
                The analysis returned no recommendations. Try again or open an
                issue if this keeps happening.
              </p>
            ) : (
              <ul className="wb-advisor-list">
                {items.map((it) => <AdvisorRow key={it.id} item={it} />)}
              </ul>
            )}
          </section>
        )}
      </div>
    </div>
  );
}

function AdvisorRow({ item }: { item: AdvisorItem }) {
  const setScope = useAdvisorStore((s) => s.setScope);
  const create = useAdvisorStore((s) => s.create);
  const skip = useAdvisorStore((s) => s.skip);
  const [open, setOpen] = useState(false);

  const scope = item.scopeOverride ?? item.scope;
  const dimmed = item.status === "skipped" || item.status === "created";

  return (
    <li className={`wb-advisor ${dimmed ? "dimmed" : ""}`}>
      <div className="wb-advisor-head" onClick={() => setOpen((v) => !v)}>
        <span className="caret">{open ? "▾" : "▸"}</span>
        <div className="wb-advisor-text">
          <div className="wb-advisor-name">{item.name}</div>
          <div className="wb-advisor-desc">{item.description}</div>
        </div>
        <StatusBadge status={item.status} />
      </div>

      {open && (
        <div className="wb-advisor-body">
          <p className="wb-advisor-why"><strong>why:</strong> {item.whyItFits}</p>

          <div className="wb-advisor-controls">
            <label className="wb-advisor-scope">
              <span>scope</span>
              <select
                value={scope}
                onChange={(e) => setScope(item.id, e.target.value as "project" | "user")}
                disabled={item.status !== "proposed" && item.status !== "error"}
              >
                <option value="project">project (.claude/skills)</option>
                <option value="user">user (~/.claude/skills)</option>
              </select>
            </label>

            <div className="wb-advisor-actions">
              {(item.status === "proposed" || item.status === "error") && (
                <>
                  <button className="wb-link" onClick={() => skip(item.id)}>skip</button>
                  <button
                    className="wb-cta wb-cta-sm"
                    onClick={() => create(item.id)}
                  >
                    Create
                  </button>
                </>
              )}
              {item.status === "creating" && <span className="wb-hint">creating…</span>}
              {item.status === "created" && item.createdPath && (
                <span className="wb-hint" title={item.createdPath}>
                  ✓ created
                </span>
              )}
              {item.status === "skipped" && <span className="wb-hint">skipped</span>}
            </div>
          </div>

          {item.errorMessage && (
            <p className="wb-hint" style={{ color: "#ff8585", whiteSpace: "pre-wrap" }}>
              {item.errorMessage}
            </p>
          )}

          <details className="wb-advisor-preview">
            <summary>SKILL.md preview</summary>
            <pre className="wb-advisor-md">{item.skillMdContent}</pre>
          </details>
        </div>
      )}
    </li>
  );
}

function formatElapsed(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}m ${s.toString().padStart(2, "0")}s`;
}

function StatusBadge({ status }: { status: AdvisorItem["status"] }) {
  if (status === "proposed") return null;
  const label =
    status === "created" ? "created" :
    status === "creating" ? "…" :
    status === "skipped" ? "skipped" :
    "error";
  return <span className={`wb-advisor-badge status-${status}`}>{label}</span>;
}
