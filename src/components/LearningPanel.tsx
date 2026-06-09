import { useEffect, useState } from "react";

import { useLearningStore, type LearningItem } from "../stores/learningStore";

export function LearningPanel() {
  const status = useLearningStore((s) => s.status);
  const items = useLearningStore((s) => s.items);
  const errorMessage = useLearningStore((s) => s.errorMessage);
  const eventsAnalyzed = useLearningStore((s) => s.eventsAnalyzed);
  const reflect = useLearningStore((s) => s.reflect);
  const reset = useLearningStore((s) => s.reset);
  const autoEnabled = useLearningStore((s) => s.autoEnabled);
  const setAutoEnabled = useLearningStore((s) => s.setAutoEnabled);
  const lastWasAuto = useLearningStore((s) => s.lastWasAuto);

  // Elapsed-time counter while reflecting, so the wait reads as live progress.
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (status !== "reflecting") { setElapsed(0); return; }
    setElapsed(0);
    const started = Date.now();
    const t = setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => clearInterval(t);
  }, [status]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">learning</span>
        <span className="spacer" />
        <button
          className={`workbench-action wb-learning-auto ${autoEnabled ? "on" : ""}`}
          onClick={() => setAutoEnabled(!autoEnabled)}
          title={
            autoEnabled
              ? "Auto-reflect is ON — reflects on its own as activity builds up. Click to turn off."
              : "Auto-reflect is OFF — only reflects when you click. Click to turn on."
          }
        >
          {autoEnabled ? "auto ⚡" : "auto"}
        </button>
        {status === "results" && (
          <button className="workbench-action" onClick={reset} title="Clear results">×</button>
        )}
        <button
          className="workbench-action"
          onClick={reflect}
          disabled={status === "reflecting"}
          title="Reflect on recent activity and suggest improvements"
        >
          {status === "reflecting" ? "…" : "↻"}
        </button>
      </div>

      <div className="workbench-body">
        {status === "idle" && (
          <section className="wb-section">
            <p className="wb-hint">
              Learning mode reflects on what you've actually been doing in this
              project — the prompts you've sent and the checkpoints they made —
              and proposes skills to automate repeated work, memories worth
              keeping, and friction worth fixing. Nothing is written until you
              confirm.
            </p>
            <p className="wb-hint">
              {autoEnabled
                ? "Auto-reflect is on — it'll surface suggestions on its own as you work. You can also run it now:"
                : "Auto-reflect is off — run it whenever you like:"}
            </p>
            <button className="wb-cta" onClick={reflect}>Reflect on recent activity</button>
          </section>
        )}

        {status === "reflecting" && (
          <section className="wb-section">
            <div className="wb-working">
              <span className="wb-spinner" />
              <div className="wb-working-text">
                <div className="wb-working-title">
                  Reflecting… <span className="wb-working-elapsed">{formatElapsed(elapsed)}</span>
                </div>
                <div className="wb-working-sub">
                  Reviewing your recent activity with Claude in the background
                  (usually 30–90s). You can keep working — suggestions land here.
                </div>
              </div>
            </div>
          </section>
        )}

        {status === "error" && (
          <section className="wb-section">
            <div className="wb-section-title">reflection failed</div>
            <p className="wb-hint" style={{ whiteSpace: "pre-wrap" }}>
              {errorMessage}
            </p>
            <button className="wb-cta" onClick={reflect}>Retry</button>
          </section>
        )}

        {status === "results" && (
          <section className="wb-section">
            <div className="wb-section-title">
              suggestions
              <span className="wb-count">{items.length}</span>
              {lastWasAuto && <span className="wb-learning-auto-tag" title="Triggered automatically by recent activity">auto</span>}
            </div>
            {items.length === 0 ? (
              <p className="wb-hint">
                {eventsAnalyzed === 0
                  ? "No activity recorded yet. Use the integrated terminal for a while — every prompt you send is captured — then reflect again."
                  : `Reviewed ${eventsAnalyzed} events but found nothing worth suggesting yet. Try again after more work.`}
              </p>
            ) : (
              <>
                <p className="wb-hint" style={{ marginTop: 0 }}>
                  From {eventsAnalyzed} recent events.
                </p>
                <ul className="wb-advisor-list">
                  {items.map((it) => <LearningRow key={it.id} item={it} />)}
                </ul>
              </>
            )}
          </section>
        )}
      </div>
    </div>
  );
}

function LearningRow({ item }: { item: LearningItem }) {
  const apply = useLearningStore((s) => s.apply);
  const skip = useLearningStore((s) => s.skip);
  const [open, setOpen] = useState(false);

  const dimmed = item.status === "skipped" || item.status === "applied";
  const canApply = item.kind !== "friction";
  const previewContent =
    item.kind === "skill" ? item.skillMdContent : item.kind === "memory" ? item.memoryContent : undefined;
  const previewLabel = item.kind === "skill" ? "SKILL.md preview" : "memory preview";
  const applyLabel = item.kind === "skill" ? "Create skill" : "Save memory";

  return (
    <li className={`wb-advisor ${dimmed ? "dimmed" : ""}`}>
      <div className="wb-advisor-head" onClick={() => setOpen((v) => !v)}>
        <span className="caret">{open ? "▾" : "▸"}</span>
        <div className="wb-advisor-text">
          <div className="wb-advisor-name">
            <KindTag kind={item.kind} /> {item.title}
          </div>
          <div className="wb-advisor-desc">{item.rationale}</div>
        </div>
        <StatusBadge status={item.status} />
      </div>

      {open && (
        <div className="wb-advisor-body">
          {item.evidence.length > 0 && (
            <div className="wb-advisor-why">
              <strong>evidence:</strong>
              <ul className="wb-learning-evidence">
                {item.evidence.map((e, i) => <li key={i}>{e}</li>)}
              </ul>
            </div>
          )}

          <div className="wb-advisor-controls">
            <span className="wb-hint" style={{ margin: 0 }}>
              {item.kind === "friction"
                ? "Report only — nothing to apply."
                : item.kind === "skill"
                  ? "Writes to .claude/skills/"
                  : "Writes to this project's memory"}
            </span>

            <div className="wb-advisor-actions">
              {(item.status === "proposed" || item.status === "error") && (
                <>
                  <button className="wb-link" onClick={() => skip(item.id)}>
                    {canApply ? "skip" : "dismiss"}
                  </button>
                  {canApply && (
                    <button className="wb-cta wb-cta-sm" onClick={() => apply(item.id)}>
                      {applyLabel}
                    </button>
                  )}
                </>
              )}
              {item.status === "applying" && <span className="wb-hint">applying…</span>}
              {item.status === "applied" && item.appliedPath && (
                <span className="wb-hint" title={item.appliedPath}>✓ applied</span>
              )}
              {item.status === "skipped" && <span className="wb-hint">dismissed</span>}
            </div>
          </div>

          {item.errorMessage && (
            <p className="wb-hint" style={{ color: "#ff8585", whiteSpace: "pre-wrap" }}>
              {item.errorMessage}
            </p>
          )}

          {previewContent && (
            <details className="wb-advisor-preview">
              <summary>{previewLabel}</summary>
              <pre className="wb-advisor-md">{previewContent}</pre>
            </details>
          )}
        </div>
      )}
    </li>
  );
}

function KindTag({ kind }: { kind: LearningItem["kind"] }) {
  return <span className={`wb-learning-kind kind-${kind}`}>{kind}</span>;
}

function formatElapsed(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}m ${s.toString().padStart(2, "0")}s`;
}

function StatusBadge({ status }: { status: LearningItem["status"] }) {
  if (status === "proposed") return null;
  const label =
    status === "applied" ? "applied" :
    status === "applying" ? "…" :
    status === "skipped" ? "dismissed" :
    "error";
  return <span className={`wb-advisor-badge status-${status}`}>{label}</span>;
}
