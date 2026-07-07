import { useEffect, useState } from "react";

import {
  useLearningStore,
  type CurationItem,
  type LearningItem,
} from "../stores/learningStore";

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

        <CurationSection />
      </div>
    </div>
  );
}

function CurationSection() {
  const status = useLearningStore((s) => s.curationStatus);
  const items = useLearningStore((s) => s.curationItems);
  const error = useLearningStore((s) => s.curationError);
  const skillsAnalyzed = useLearningStore((s) => s.skillsAnalyzed);
  const memoriesAnalyzed = useLearningStore((s) => s.memoriesAnalyzed);
  const curate = useLearningStore((s) => s.curate);
  const resetCuration = useLearningStore((s) => s.resetCuration);
  const autoEnabled = useLearningStore((s) => s.curateAutoEnabled);
  const setAutoEnabled = useLearningStore((s) => s.setCurateAutoEnabled);

  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (status !== "curating") { setElapsed(0); return; }
    const started = Date.now();
    const t = setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => clearInterval(t);
  }, [status]);

  return (
    <section className="wb-section">
      <div className="wb-section-title">
        optimize corpus
        {status === "results" && <span className="wb-count">{items.length}</span>}
        <span className="spacer" />
        <button
          className={`wb-link wb-learning-auto ${autoEnabled ? "on" : ""}`}
          onClick={() => setAutoEnabled(!autoEnabled)}
          title={
            autoEnabled
              ? "Auto-curate is ON — runs a pass on its own once the corpus grows. Click to turn off."
              : "Auto-curate is OFF — only runs when you click. Click to turn on."
          }
        >
          {autoEnabled ? "auto ⚡" : "auto"}
        </button>
        {status === "results" && (
          <button className="wb-link" onClick={resetCuration} title="Clear curation results">×</button>
        )}
        <button
          className="wb-link"
          onClick={curate}
          disabled={status === "curating"}
          title="Analyze existing skills & memories and suggest consolidations, fixes, and cleanups"
        >
          {status === "curating" ? "…" : "↻"}
        </button>
      </div>

      {status === "idle" && (
        <p className="wb-hint">
          Curation tends what you've already accumulated — it fuses overlapping
          skills/memories, flags entries pointing at code that no longer exists,
          rewrites sloppy ones, and surfaces dead weight. Suggest-only; archiving
          moves entries aside (reversible), never deletes.
        </p>
      )}

      {status === "curating" && (
        <div className="wb-working">
          <span className="wb-spinner" />
          <div className="wb-working-text">
            <div className="wb-working-title">
              Curating… <span className="wb-working-elapsed">{formatElapsed(elapsed)}</span>
            </div>
            <div className="wb-working-sub">
              Reviewing your skills & memories with Claude in the background.
            </div>
          </div>
        </div>
      )}

      {status === "error" && (
        <>
          <p className="wb-hint" style={{ whiteSpace: "pre-wrap", color: "#ff8585" }}>{error}</p>
          <button className="wb-cta" onClick={curate}>Retry</button>
        </>
      )}

      {status === "results" && (
        items.length === 0 ? (
          <p className="wb-hint">
            {skillsAnalyzed + memoriesAnalyzed < 2
              ? "Not enough skills/memories yet to consolidate. Come back once the corpus grows."
              : `Reviewed ${skillsAnalyzed} skills and ${memoriesAnalyzed} memories — the corpus looks clean. Nothing to optimize.`}
          </p>
        ) : (
          <>
            <p className="wb-hint" style={{ marginTop: 0 }}>
              From {skillsAnalyzed} skills and {memoriesAnalyzed} memories.
            </p>
            <ul className="wb-advisor-list">
              {items.map((it) => <CurationRow key={it.id} item={it} />)}
            </ul>
          </>
        )
      )}
    </section>
  );
}

function CurationRow({ item }: { item: CurationItem }) {
  const applyCuration = useLearningStore((s) => s.applyCuration);
  const skipCuration = useLearningStore((s) => s.skipCuration);
  const [open, setOpen] = useState(false);

  const dimmed = item.status === "skipped" || item.status === "applied";
  const canApply = item.action !== "rerank";
  const applyLabel =
    item.action === "merge" ? "Merge" :
    item.action === "refactor" ? "Rewrite" :
    "Archive";
  const effect =
    item.action === "merge"
      ? `Writes ${item.newName ?? "the merged entry"}, archives the rest`
      : item.action === "refactor"
        ? "Overwrites the entry in place"
        : item.action === "archive"
          ? "Moves the entry to _archived/ (reversible)"
          : "Report only — nothing to apply.";

  return (
    <li className={`wb-advisor ${dimmed ? "dimmed" : ""}`}>
      <div className="wb-advisor-head" onClick={() => setOpen((v) => !v)}>
        <span className="caret">{open ? "▾" : "▸"}</span>
        <div className="wb-advisor-text">
          <div className="wb-advisor-name">
            <CurationTag action={item.action} /> {item.title}
          </div>
          <div className="wb-advisor-desc">{item.rationale}</div>
          <div className="wb-curation-targets">
            <span className={`wb-learning-kind kind-${item.targetKind}`}>{item.targetKind}</span>
            {item.targets.map((t) => (
              <span key={t} className="wb-curation-target">{t}</span>
            ))}
          </div>
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
            <span className="wb-hint" style={{ margin: 0 }}>{effect}</span>
            <div className="wb-advisor-actions">
              {(item.status === "proposed" || item.status === "error") && (
                <>
                  <button className="wb-link" onClick={() => skipCuration(item.id)}>
                    {canApply ? "skip" : "dismiss"}
                  </button>
                  {canApply && (
                    <button className="wb-cta wb-cta-sm" onClick={() => applyCuration(item.id)}>
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

          {item.newContent && (
            <details className="wb-advisor-preview">
              <summary>{item.action === "merge" ? "merged result preview" : "rewritten preview"}</summary>
              <pre className="wb-advisor-md">{item.newContent}</pre>
            </details>
          )}
        </div>
      )}
    </li>
  );
}

function CurationTag({ action }: { action: CurationItem["action"] }) {
  return <span className={`wb-learning-kind action-${action}`}>{action}</span>;
}

function LearningRow({ item }: { item: LearningItem }) {
  const apply = useLearningStore((s) => s.apply);
  const skip = useLearningStore((s) => s.skip);
  const [open, setOpen] = useState(false);

  const dimmed = item.status === "skipped" || item.status === "applied";
  const canApply = item.kind !== "friction" && item.kind !== "hook";
  const previewContent =
    item.kind === "skill" ? item.skillMdContent
    : item.kind === "plugin" ? item.pluginSkillMd
    : item.kind === "memory" ? item.memoryContent
    : undefined;
  const previewLabel =
    item.kind === "plugin" ? "plugin SKILL.md preview"
    : item.kind === "skill" ? "SKILL.md preview"
    : "memory preview";
  const applyLabel =
    item.kind === "skill" ? "Create skill"
    : item.kind === "plugin" ? "Scaffold plugin"
    : "Save memory";

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
                : item.kind === "hook"
                  ? "Report only — wire the hook yourself in settings (hooks run commands; keep that decision human)."
                  : item.kind === "skill"
                    ? "Writes to .claude/skills/"
                    : item.kind === "plugin"
                      ? "Scaffolds ~/.claude/skills/<name>/ — auto-loads next session, shareable via marketplace"
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
