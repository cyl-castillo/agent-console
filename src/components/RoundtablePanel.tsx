import { useEffect, useRef } from "react";

import { useRoundtableStore } from "../stores/roundtableStore";
import { AGENT_PROFILES } from "../agents/profiles";
import { MarkdownText } from "./MarkdownText";
import { DiffViewer } from "./DiffViewer";
import type { RoundtableActivity, RoundtableTurn } from "../types/domain";

const CLAUDE_MODELS = AGENT_PROFILES.find((p) => p.kind === "claude")!.models;

export function RoundtablePanel() {
  const phase = useRoundtableStore((s) => s.phase);

  // Bind event listeners as soon as the panel mounts, even before a run starts,
  // so the very first turn isn't missed.
  const initListeners = useRoundtableStore((s) => s.initListeners);
  useEffect(() => {
    void initListeners();
  }, [initListeners]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">roundtable</span>
        <span className="spacer" />
        <RunControls />
      </div>
      <div className="workbench-body">
        {phase === "config" ? <ConfigForm /> : <DebateView />}
      </div>
    </div>
  );
}

function RunControls() {
  const phase = useRoundtableStore((s) => s.phase);
  const pause = useRoundtableStore((s) => s.pause);
  const resume = useRoundtableStore((s) => s.resume);
  const stop = useRoundtableStore((s) => s.stop);
  const reset = useRoundtableStore((s) => s.reset);

  if (phase === "config") return null;
  const finished = phase === "done" || phase === "stopped" || phase === "error";

  return (
    <>
      {phase === "running" && (
        <button className="workbench-action" onClick={pause} title="Pause at next turn">⏸</button>
      )}
      {phase === "paused" && (
        <button className="workbench-action" onClick={resume} title="Resume">▶</button>
      )}
      {!finished && (
        <button className="workbench-action" onClick={stop} title="Stop the debate">⏹</button>
      )}
      <button className="workbench-action" onClick={reset} title="Discard and reset">×</button>
    </>
  );
}

function ConfigForm() {
  const draft = useRoundtableStore((s) => s.draft);
  const setDraft = useRoundtableStore((s) => s.setDraft);
  const start = useRoundtableStore((s) => s.start);
  const message = useRoundtableStore((s) => s.message);

  return (
    <section className="wb-section">
      <p className="wb-hint">
        Two agents debate a question, each in its own isolated git worktree —
        free to prototype real code to back their argument. You moderate: pause,
        steer, or stop. At the end you pick a side to apply onto your working
        tree (a snapshot is taken first).
      </p>

      <label className="rt-field">
        <span>question to debate</span>
        <textarea
          className="rt-topic"
          rows={3}
          placeholder="e.g. Should the session store be normalized, or is the current denormalized shape fine? Show me."
          value={draft.topic}
          onChange={(e) => setDraft({ topic: e.target.value })}
        />
      </label>

      <div className="rt-participants">
        <ParticipantFields side="A" />
        <ParticipantFields side="B" />
      </div>

      <div className="rt-knobs">
        <label className="rt-field rt-field-sm">
          <span>max rounds</span>
          <input
            type="number"
            min={1}
            max={20}
            value={draft.maxRounds}
            onChange={(e) => setDraft({ maxRounds: Number(e.target.value) })}
          />
        </label>
        <label className="rt-field rt-field-sm">
          <span>token budget</span>
          <input
            type="number"
            min={0}
            step={50000}
            value={draft.tokenBudget}
            onChange={(e) => setDraft({ tokenBudget: Number(e.target.value) })}
          />
        </label>
      </div>

      <label className="rt-toggle" title="Edits-only is safe; full tools lets agents run Bash (still sandboxed to their worktree).">
        <input
          type="checkbox"
          checked={draft.fullTools}
          onChange={(e) => setDraft({ fullTools: e.target.checked })}
        />
        <span>
          {draft.fullTools
            ? "Full tools — agents may run Bash (sandboxed to their worktree)"
            : "Edits only — agents may edit files, no shell"}
        </span>
      </label>

      {message && <p className="wb-hint" style={{ color: "#ff8585" }}>{message}</p>}

      <button className="wb-cta" onClick={start} disabled={!draft.topic.trim()}>
        Start debate
      </button>
    </section>
  );
}

function ParticipantFields({ side }: { side: "A" | "B" }) {
  const draft = useRoundtableStore((s) => s.draft);
  const setDraft = useRoundtableStore((s) => s.setDraft);
  const isA = side === "A";
  const name = isA ? draft.nameA : draft.nameB;
  const model = isA ? draft.modelA : draft.modelB;
  const persona = isA ? draft.personaA : draft.personaB;

  return (
    <div className={`rt-participant rt-side-${side.toLowerCase()}`}>
      <div className="rt-participant-head">
        <span className="rt-dot" /> participant {side}
      </div>
      <label className="rt-field rt-field-sm">
        <span>name</span>
        <input
          value={name}
          onChange={(e) => setDraft(isA ? { nameA: e.target.value } : { nameB: e.target.value })}
        />
      </label>
      <label className="rt-field rt-field-sm">
        <span>model</span>
        <select
          value={model}
          onChange={(e) => setDraft(isA ? { modelA: e.target.value } : { modelB: e.target.value })}
        >
          {CLAUDE_MODELS.map((m) => (
            <option key={m.value} value={m.value}>{m.label} · {m.intent}</option>
          ))}
        </select>
      </label>
      <label className="rt-field rt-field-sm">
        <span>stance (optional)</span>
        <input
          placeholder={isA ? "e.g. argue for simplicity" : "e.g. argue for robustness"}
          value={persona}
          onChange={(e) =>
            setDraft(isA ? { personaA: e.target.value } : { personaB: e.target.value })
          }
        />
      </label>
    </div>
  );
}

function DebateView() {
  const turns = useRoundtableStore((s) => s.turns);
  const activities = useRoundtableStore((s) => s.activities);
  const phase = useRoundtableStore((s) => s.phase);
  const round = useRoundtableStore((s) => s.round);
  const totalTokens = useRoundtableStore((s) => s.totalTokens);
  const approxCostUsd = useRoundtableStore((s) => s.approxCostUsd);
  const message = useRoundtableStore((s) => s.message);
  const draft = useRoundtableStore((s) => s.draft);
  const diffSide = useRoundtableStore((s) => s.diffSide);

  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [turns.length, activities.length]);

  // Activities whose turn hasn't completed yet = the in-flight turn's live feed.
  const completedKeys = new Set(turns.map((t) => `${t.side}-${t.round}`));
  const live = activities.filter((a) => !completedKeys.has(`${a.side}-${a.round}`));
  const liveSide = live.length ? live[live.length - 1].side : null;

  return (
    <section className="rt-debate">
      <div className="rt-meta">
        <span className={`rt-phase rt-phase-${phase}`}>{phase}</span>
        <span className="rt-meta-sep">·</span>
        <span>round {round}/{draft.maxRounds}</span>
        <span className="rt-meta-sep">·</span>
        <span>{formatTokens(totalTokens)} tok</span>
        {draft.tokenBudget > 0 && (
          <span className="rt-budget">
            <span
              className="rt-budget-fill"
              style={{ width: `${Math.min(100, (totalTokens / draft.tokenBudget) * 100)}%` }}
            />
          </span>
        )}
        {approxCostUsd > 0 && (
          <>
            <span className="rt-meta-sep">·</span>
            <span title="approx cumulative cost">${approxCostUsd.toFixed(3)}</span>
          </>
        )}
      </div>

      <div className="rt-topic-banner" title={draft.topic}>{draft.topic}</div>

      <div className="rt-transcript" ref={scrollRef}>
        {turns.map((t, i) => (
          <TurnBubble
            key={i}
            turn={t}
            activities={activities.filter((a) => a.side === t.side && a.round === t.round)}
          />
        ))}

        {phase === "running" && (
          <div className={`rt-turn rt-live rt-side-${liveSide ?? "a"}`}>
            <div className="rt-turn-head">
              <span className="rt-dot" />
              <span className="rt-turn-name">
                {liveSide === "b" ? draft.nameB : draft.nameA}
              </span>
              <span className="spacer" />
              <span className="rt-thinking-inline"><span className="wb-spinner" /> working…</span>
            </div>
            {live.length > 0 ? (
              <ActivityFeed items={live} showText />
            ) : (
              <div className="rt-activity-empty">starting turn…</div>
            )}
          </div>
        )}
      </div>

      {message && <div className="rt-banner">{message}</div>}

      {diffSide && <DiffOverlay />}

      <Moderator />

      {(phase === "done" || phase === "stopped") && <WinnerBar />}
    </section>
  );
}

function ActivityFeed({ items, showText }: { items: RoundtableActivity[]; showText: boolean }) {
  const shown = showText ? items : items.filter((a) => a.kind !== "text");
  if (shown.length === 0) return null;
  return (
    <div className="rt-activity">
      {shown.map((a, i) => {
        if (a.kind === "tool") {
          return (
            <div key={i} className="rt-act rt-act-tool">
              <span className="rt-act-name">▸ {a.label}</span>
              {a.text && <span className="rt-act-detail">{a.text}</span>}
            </div>
          );
        }
        if (a.kind === "thinking") {
          return <div key={i} className="rt-act rt-act-thinking">🧠 {a.text}</div>;
        }
        return (
          <div key={i} className="rt-act rt-act-text">
            <MarkdownText content={a.text} />
          </div>
        );
      })}
    </div>
  );
}

function TurnBubble({ turn, activities }: { turn: RoundtableTurn; activities: RoundtableActivity[] }) {
  const openDiff = useRoundtableStore((s) => s.openDiff);
  const steps = activities.filter((a) => a.kind !== "text");
  return (
    <div className={`rt-turn rt-side-${turn.side}`}>
      <div className="rt-turn-head">
        <span className="rt-dot" />
        <span className="rt-turn-name">{turn.name}</span>
        <span className="rt-turn-model">{turn.model}</span>
        <span className="spacer" />
        <span className="rt-turn-round">r{turn.round}</span>
      </div>
      {steps.length > 0 && (
        <details className="rt-steps">
          <summary>{steps.length} step{steps.length === 1 ? "" : "s"} — what it did</summary>
          <ActivityFeed items={steps} showText={false} />
        </details>
      )}
      <div className="rt-turn-body">
        <MarkdownText content={turn.text} />
      </div>
      {turn.diffStat && (
        <button className="rt-diff-stat" onClick={() => openDiff(turn.side)} title="View full diff">
          <pre>{turn.diffStat}</pre>
        </button>
      )}
    </div>
  );
}

function DiffOverlay() {
  const diffSide = useRoundtableStore((s) => s.diffSide)!;
  const diff = useRoundtableStore((s) => s.diff);
  const loading = useRoundtableStore((s) => s.diffLoading);
  const closeDiff = useRoundtableStore((s) => s.closeDiff);
  const apply = useRoundtableStore((s) => s.apply);

  return (
    <div className="rt-diff-overlay">
      <div className="rt-diff-head">
        <span>side {diffSide.toUpperCase()} — working diff vs HEAD</span>
        <span className="spacer" />
        <button className="wb-cta wb-cta-sm" onClick={() => apply(diffSide)}>Apply this side</button>
        <button className="workbench-action" onClick={closeDiff}>×</button>
      </div>
      <div className="rt-diff-scroll">
        {loading ? (
          <div className="rt-thinking"><span className="wb-spinner" /> loading diff…</div>
        ) : (
          <DiffViewer diff={diff} empty="This side hasn't changed any files." />
        )}
      </div>
    </div>
  );
}

function Moderator() {
  const phase = useRoundtableStore((s) => s.phase);
  const injectDraft = useRoundtableStore((s) => s.injectDraft);
  const setInjectDraft = useRoundtableStore((s) => s.setInjectDraft);
  const inject = useRoundtableStore((s) => s.inject);

  if (phase === "done" || phase === "stopped" || phase === "error") return null;

  return (
    <div className="rt-moderator">
      <input
        placeholder="Interject as moderator — steer the debate (applied before the next turn)…"
        value={injectDraft}
        onChange={(e) => setInjectDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && injectDraft.trim()) void inject();
        }}
      />
      <button className="wb-cta wb-cta-sm" onClick={() => void inject()} disabled={!injectDraft.trim()}>
        Send
      </button>
    </div>
  );
}

function WinnerBar() {
  const apply = useRoundtableStore((s) => s.apply);
  const openDiff = useRoundtableStore((s) => s.openDiff);
  const appliedSide = useRoundtableStore((s) => s.appliedSide);
  const appliedSnapshot = useRoundtableStore((s) => s.appliedSnapshot);
  const draft = useRoundtableStore((s) => s.draft);

  if (appliedSide) {
    return (
      <div className="rt-winner rt-applied">
        ✓ Applied side {appliedSide.toUpperCase()} onto your working tree.
        {appliedSnapshot && (
          <span className="rt-snap" title={appliedSnapshot}>
            {" "}snapshot {appliedSnapshot.slice(0, 8)} taken first
          </span>
        )}
      </div>
    );
  }

  return (
    <div className="rt-winner">
      <span>pick a side to keep:</span>
      <button className="rt-pick rt-side-a" onClick={() => openDiff("a")}>review {draft.nameA}</button>
      <button className="rt-pick rt-side-a" onClick={() => apply("a")}>apply A</button>
      <button className="rt-pick rt-side-b" onClick={() => openDiff("b")}>review {draft.nameB}</button>
      <button className="rt-pick rt-side-b" onClick={() => apply("b")}>apply B</button>
    </div>
  );
}

function formatTokens(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return `${n}`;
}
