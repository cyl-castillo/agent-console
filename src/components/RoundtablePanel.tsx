import { useEffect, useRef, useState } from "react";

import {
  useRoundtableStore,
  modelsFor,
  CONNECTOR_ROLES,
  type RtParticipantDraft,
} from "../stores/roundtableStore";
import { useChangesStore } from "../stores/changesStore";
import { AGENT_PROFILES } from "../agents/profiles";
import { MarkdownText } from "./MarkdownText";
import type {
  ConnectorPendingJob,
  ConnectorQuestion,
  ConnectorTask,
  ConnectorView,
  RoundtableActivity,
  RoundtableTurn,
} from "../types/domain";

// Stable accent per participant, so each voice is recognizable in the feed.
const PALETTE = ["#6aa9ff", "#ff9e64", "#9ece6a", "#bb9af7", "#f7768e", "#7dcfff"];
function authorColor(id: string): string {
  if (id === "human") return "#c0caf5";
  const i = parseInt(id.replace(/\D/g, ""), 10) - 1;
  return PALETTE[(Number.isFinite(i) && i >= 0 ? i : 0) % PALETTE.length];
}

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
        <span className="workbench-title">room</span>
        <span className="spacer" />
        <RunControls />
      </div>
      <div className="workbench-body">{phase === "config" ? <ConfigForm /> : <RoomView />}</div>
    </div>
  );
}

function RunControls() {
  const phase = useRoundtableStore((s) => s.phase);
  const readOnly = useRoundtableStore((s) => s.readOnly);
  const pause = useRoundtableStore((s) => s.pause);
  const resume = useRoundtableStore((s) => s.resume);
  const stop = useRoundtableStore((s) => s.stop);
  const reset = useRoundtableStore((s) => s.reset);

  if (phase === "config") return null;

  // Viewing a saved room: the only action is to close the viewer (keeps it on disk).
  if (readOnly) {
    return (
      <button className="workbench-action" onClick={reset} title="Close (keeps the saved room)">
        ×
      </button>
    );
  }

  const finished = phase === "done" || phase === "stopped" || phase === "error";

  return (
    <>
      {phase === "running" && (
        <button className="workbench-action" onClick={pause} title="Pause at next turn">
          ⏸
        </button>
      )}
      {phase === "paused" && (
        <button className="workbench-action" onClick={resume} title="Resume">
          ▶
        </button>
      )}
      {!finished && (
        <button className="workbench-action" onClick={stop} title="Stop the conversation">
          ⏹
        </button>
      )}
      <button className="workbench-action" onClick={reset} title="Discard and reset">
        ×
      </button>
    </>
  );
}

function ConfigForm() {
  const draft = useRoundtableStore((s) => s.draft);
  const setDraft = useRoundtableStore((s) => s.setDraft);
  const addParticipant = useRoundtableStore((s) => s.addParticipant);
  const start = useRoundtableStore((s) => s.start);
  const message = useRoundtableStore((s) => s.message);

  // Agents can only edit inside a git worktree, so "let them edit" needs a repo.
  // Reflect that up front: when the open folder isn't a git repo, disable the
  // toggle instead of letting the room start and silently fall back to read-only.
  const isRepo = useChangesStore((s) => s.status?.isRepo);
  const refreshGit = useChangesStore((s) => s.refresh);
  useEffect(() => {
    if (isRepo === undefined) void refreshGit();
  }, [isRepo, refreshGit]);
  const noRepo = isRepo === false;
  const updateParticipant = useRoundtableStore((s) => s.updateParticipant);

  // Turning job mode on needs an organizer. If nobody holds connector roles
  // yet, seed the classic trio (organizer, implementer, reviewer…) in roster
  // order so the form starts valid; existing choices are left alone.
  const setJobMode = (on: boolean) => {
    setDraft({ jobMode: on });
    if (!on || draft.participants.some((p) => p.roles.length > 0)) return;
    const seed = ["organizer", "implementer", "reviewer"];
    draft.participants.forEach((p, i) => {
      updateParticipant(p.id, { roles: [seed[i] ?? "implementer"] });
    });
  };

  return (
    <section className="wb-section">
      <p className="wb-hint">
        You plus a room of agents (Claude and/or Codex) hold one shared conversation about a
        problem. Each takes turns; everyone sees what the others — and you — said. They can read the
        open project to ground their reasoning but won't edit anything. Steer anytime by posting a
        message.
      </p>

      <label className="rt-field">
        <span>the problem</span>
        <textarea
          className="rt-topic"
          rows={3}
          placeholder="e.g. Our session store re-renders the whole list on every keystroke. What's the cleanest fix given the current shape?"
          value={draft.problem}
          onChange={(e) => setDraft({ problem: e.target.value })}
        />
      </label>

      <div className="rt-roster-config">
        {draft.participants.map((p) => (
          <ParticipantRow key={p.id} p={p} canRemove={draft.participants.length > 2} />
        ))}
        <button className="rt-add-participant" onClick={addParticipant}>
          + add participant
        </button>
      </div>

      <div className="rt-knobs">
        <label className="rt-field rt-field-sm">
          <span>max turns</span>
          <input
            type="number"
            min={1}
            max={60}
            value={draft.maxTurns}
            onChange={(e) => setDraft({ maxTurns: Number(e.target.value) })}
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

      <label className="rt-toggle" style={noRepo ? { opacity: 0.6 } : undefined}>
        <input
          type="checkbox"
          checked={draft.allowEdits && !noRepo}
          disabled={noRepo}
          onChange={(e) => setDraft({ allowEdits: e.target.checked })}
        />
        <span className="rt-toggle-text">
          <span className="rt-toggle-title">Let agents edit the code</span>
          <span className="rt-toggle-hint">
            {noRepo
              ? "Unavailable — the open folder isn't a git repo. Editing needs a repo with at least one commit; conversation still works."
              : draft.allowEdits
                ? "On — they work in an isolated worktree on a room/… branch; you review and merge. Your files stay untouched."
                : "Off — conversation only, read-only."}
          </span>
        </span>
      </label>

      <label className="rt-toggle">
        <input
          type="checkbox"
          checked={draft.jobMode}
          onChange={(e) => setJobMode(e.target.checked)}
        />
        <span className="rt-toggle-text">
          <span className="rt-toggle-title">Run as a job (the organizer drives it)</span>
          <span className="rt-toggle-hint">
            {draft.jobMode
              ? "On — the organizer gets the objective and delegates through the connector; the room runs only the turns the queue asks for, then reviews (if required) and closes. Exactly one participant needs the organizer role."
              : "Off — a round-robin conversation; agents may still delegate or ask you questions."}
          </span>
        </span>
      </label>

      {draft.jobMode && (
        <div className="rt-knobs">
          <label className="rt-toggle rt-toggle-inline">
            <input
              type="checkbox"
              checked={draft.reviewRequired}
              onChange={(e) => setDraft({ reviewRequired: e.target.checked })}
            />
            <span className="rt-toggle-text">
              <span className="rt-toggle-title">Require a review</span>
              <span className="rt-toggle-hint">
                A participant with the reviewer role approves the result before the job closes;
                "changes" sends a correction back.
              </span>
            </span>
          </label>
          <label className="rt-field rt-field-sm">
            <span>max corrections</span>
            <input
              type="number"
              min={0}
              max={10}
              value={draft.maxCorrections}
              onChange={(e) => setDraft({ maxCorrections: Number(e.target.value) })}
            />
          </label>
        </div>
      )}

      {message && (
        <p className="wb-hint" style={{ color: "#ff8585" }}>
          {message}
        </p>
      )}

      <button className="wb-cta" onClick={start} disabled={!draft.problem.trim()}>
        {draft.jobMode ? "Start job" : "Start conversation"}
      </button>
    </section>
  );
}

function ParticipantRow({ p, canRemove }: { p: RtParticipantDraft; canRemove: boolean }) {
  const update = useRoundtableStore((s) => s.updateParticipant);
  const remove = useRoundtableStore((s) => s.removeParticipant);
  const models = modelsFor(p.engine);

  // Claude and Codex expose different "model" values (aliases vs effort levels),
  // so switching engine resets the model to the new engine's first preset.
  const setEngine = (engine: "claude" | "codex") => {
    const firstModel = modelsFor(engine)[0]?.value ?? "";
    update(p.id, { engine, model: firstModel });
  };

  return (
    <div className="rt-participant" style={{ borderLeftColor: authorColor(p.id) }}>
      <div className="rt-participant-grid">
        <label className="rt-field rt-field-sm">
          <span>name</span>
          <input value={p.name} onChange={(e) => update(p.id, { name: e.target.value })} />
        </label>
        <label className="rt-field rt-field-sm">
          <span>engine</span>
          <select
            value={p.engine}
            onChange={(e) => setEngine(e.target.value as "claude" | "codex")}
          >
            {AGENT_PROFILES.map((prof) => (
              <option key={prof.kind} value={prof.kind}>
                {prof.icon} {prof.label}
              </option>
            ))}
          </select>
        </label>
        <label className="rt-field rt-field-sm">
          <span>{p.engine === "codex" ? "effort" : "model"}</span>
          <select value={p.model} onChange={(e) => update(p.id, { model: e.target.value })}>
            {models.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
        {canRemove && (
          <button className="rt-remove-participant" onClick={() => remove(p.id)} title="Remove">
            ×
          </button>
        )}
      </div>
      <label className="rt-field rt-field-sm">
        <span>role (optional)</span>
        <input
          placeholder="e.g. the skeptic / the implementer / focus on edge cases"
          value={p.role}
          onChange={(e) => update(p.id, { role: e.target.value })}
        />
      </label>
      <div
        className="rt-roles"
        title="Connector roles: what this agent can be delegated by its peers, and whether it may record a review. None = plain assistant."
      >
        {CONNECTOR_ROLES.map((r) => {
          const on = p.roles.includes(r);
          return (
            <button
              key={r}
              type="button"
              className={`rt-role-chip ${on ? "rt-role-chip-on" : ""}`}
              onClick={() =>
                update(p.id, { roles: on ? p.roles.filter((x) => x !== r) : [...p.roles, r] })
              }
            >
              {r}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function RoomView() {
  const turns = useRoundtableStore((s) => s.turns);
  const activities = useRoundtableStore((s) => s.activities);
  const phase = useRoundtableStore((s) => s.phase);
  const readOnly = useRoundtableStore((s) => s.readOnly);
  const workingRoom = useRoundtableStore((s) => s.workingRoom);
  const problem = useRoundtableStore((s) => s.problem);
  const turn = useRoundtableStore((s) => s.turn);
  const targetTurns = useRoundtableStore((s) => s.targetTurns);
  const totalTokens = useRoundtableStore((s) => s.totalTokens);
  const approxCostUsd = useRoundtableStore((s) => s.approxCostUsd);
  const message = useRoundtableStore((s) => s.message);
  const draft = useRoundtableStore((s) => s.draft);
  const roster = useRoundtableStore((s) => s.roster);
  const jobMode = useRoundtableStore((s) => s.jobMode);
  const originRoomId = useRoundtableStore((s) => s.originRoomId);
  // Streamed text is coalesced into the SAME activity object, so activities.length
  // doesn't change mid-message — lastActivityAt does (every chunk), so it's the
  // signal that keeps the feed pinned to the bottom while an answer streams in.
  const lastActivityAt = useRoundtableStore((s) => s.lastActivityAt);

  const scrollRef = useRef<HTMLDivElement>(null);
  // Stick to the bottom only while the user is already there, so scrolling up to
  // read an earlier message isn't hijacked by every stream chunk.
  const stickRef = useRef(true);
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  };
  useEffect(() => {
    if (!stickRef.current) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [turns.length, activities.length, lastActivityAt]);

  const n = Math.max(1, roster.length);
  // Activities whose AI turn hasn't completed yet = the in-flight turn's feed.
  const completedKeys = new Set(
    turns.filter((t) => !t.isHuman).map((t) => `${t.authorId}-${t.turn}`),
  );
  const live = activities.filter((a) => !completedKeys.has(`${a.authorId}-${a.turn}`));
  // Who's up: trust streaming activity; before it arrives, infer from how many
  // AI turns have completed (round-robin over the launched roster order).
  const aiTurns = turns.filter((t) => !t.isHuman).length;
  const liveAuthorId = live.length ? live[live.length - 1].authorId : `p${(aiTurns % n) + 1}`;
  const liveParticipant = roster.find((p) => p.id === liveAuthorId) ?? roster[0];

  return (
    <section className="rt-debate">
      <div className="rt-meta">
        <span className={`rt-phase rt-phase-${readOnly ? "done" : phase}`}>
          {readOnly ? "saved · read-only" : phase}
        </span>
        {!readOnly && draft.allowEdits && (
          <>
            <span className="rt-meta-sep">·</span>
            <span
              className="rt-editing"
              title="Agents edit in an isolated worktree on a room/… branch; review & merge when done"
            >
              ✎ editing
            </span>
          </>
        )}
        {jobMode && (
          <>
            <span className="rt-meta-sep">·</span>
            <span
              className="rt-editing"
              title="Job mode: the organizer drives the work through the connector; the room closes when nothing is pending"
            >
              ⚙ job
            </span>
          </>
        )}
        {originRoomId && (
          <>
            <span className="rt-meta-sep">·</span>
            <span title={`Approved from room ${originRoomId}`}>↳ follow-up</span>
          </>
        )}
        <span className="rt-meta-sep">·</span>
        <span>
          turn {turn}/{targetTurns || draft.maxTurns}
        </span>
        <span className="rt-meta-sep">·</span>
        <span>{formatTokens(totalTokens)} tok</span>
        {!readOnly && draft.tokenBudget > 0 && (
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
            <span title="approx cumulative cost (Claude turns only — Codex reports no cost)">
              ${approxCostUsd.toFixed(3)}
            </span>
          </>
        )}
      </div>

      <div className="rt-topic-banner" title={problem}>
        {problem}
      </div>

      <div className="rt-roster">
        {roster.map((p) => (
          <RosterChip
            key={p.id}
            id={p.id}
            name={p.name}
            model={p.model}
            engine={p.engine}
            active={phase === "running" && liveAuthorId === p.id}
          />
        ))}
      </div>

      <div className="rt-transcript" ref={scrollRef} onScroll={onScroll}>
        {turns.map((t, i) => (
          <MessageBubble
            key={i}
            turn={t}
            activities={
              t.isHuman
                ? []
                : activities.filter((a) => a.authorId === t.authorId && a.turn === t.turn)
            }
          />
        ))}

        {phase === "running" && (
          <div className="rt-turn rt-live" style={{ borderLeftColor: authorColor(liveAuthorId) }}>
            <div className="rt-turn-head">
              <span className="rt-dot" style={{ background: authorColor(liveAuthorId) }} />
              <span className="rt-turn-name">{liveParticipant?.name ?? liveAuthorId}</span>
              <span className="spacer" />
              <LiveStatus live={live} />
            </div>
            {live.length > 0 ? (
              <ActivityFeed items={live} showText />
            ) : (
              <div className="rt-activity-empty">starting turn…</div>
            )}
          </div>
        )}
      </div>

      <ConnectorBlock />

      {!readOnly && workingRoom && <CoworkBar />}

      {message && (
        <div className={`rt-banner ${phase === "error" ? "rt-banner-error" : "rt-banner-info"}`}>
          {message}
        </div>
      )}

      {readOnly ? <SavedRoomFooter /> : <HumanInput />}
    </section>
  );
}

/// Cowork with human colleagues over the git remote — the inbound/outbound
/// bridge. Inline (no popover) to respect the 240px sidebar clipping. Only shown
/// for a live working room (it edits a room/… branch). "Share" pushes the branch
/// + transcript and surfaces the MR/PR link; "Sync" pulls a colleague's commits
/// into the worktree and reports any conflicts.
function CoworkBar() {
  const share = useRoundtableStore((s) => s.share);
  const sync = useRoundtableStore((s) => s.sync);
  const busy = useRoundtableStore((s) => s.coworkBusy);
  const result = useRoundtableStore((s) => s.coworkResult);
  const clear = useRoundtableStore((s) => s.clearCowork);

  return (
    <div className="rt-cowork">
      <div className="rt-cowork-actions">
        <span
          className="rt-cowork-label"
          title="Connect with colleagues working on the same problem, over your git remote"
        >
          cowork
        </span>
        <span className="spacer" />
        <button
          className="wb-cta wb-cta-sm"
          onClick={() => void share()}
          disabled={!!busy}
          title="Push this room's branch (with its transcript) to the remote and get an MR/PR link"
        >
          {busy === "share" ? "Sharing…" : "Share / open MR ▸"}
        </button>
        <button
          className="wb-cta wb-cta-sm"
          onClick={() => void sync()}
          disabled={!!busy}
          title="Fetch a colleague's commits from the remote room branch and merge them into the worktree"
        >
          {busy === "sync" ? "Syncing…" : "⭳ Sync colleague work"}
        </button>
      </div>

      {result && (
        <div
          className={`rt-banner ${result.kind === "sync" && result.conflicts.length ? "rt-banner-error" : "rt-banner-info"}`}
        >
          <span>{result.message}</span>
          {result.kind === "share" && result.prUrl && (
            <>
              {" "}
              <a href={result.prUrl} target="_blank" rel="noreferrer">
                Open MR/PR ↗
              </a>
            </>
          )}
          <button className="rt-cowork-dismiss" onClick={clear} title="Dismiss">
            ×
          </button>
        </div>
      )}
    </div>
  );
}

/// Footer for a saved room being viewed read-only: a single affordance to bring
/// it back to life. Resuming flips the panel into the live "awaiting" state.
function SavedRoomFooter() {
  const resumeRoom = useRoundtableStore((s) => s.resumeRoom);
  return (
    <div className="rt-moderator">
      <span className="rt-readonly-note">
        Saved room · read-only. Reading the engines' prior sessions isn't guaranteed.
      </span>
      <span className="spacer" />
      <button className="wb-cta wb-cta-sm rt-continue" onClick={() => void resumeRoom()}>
        Continue conversation ▸
      </button>
    </div>
  );
}

// The live turn's header: WHAT it's doing now, HOW LONG it's run, and whether
// it's STILL MOVING. The last can't come from elapsed time — a long read emits
// no activity while it runs — so the gap since the last activity is the signal.
const STALE_AFTER_MS = 15_000;

function LiveStatus({ live }: { live: RoundtableActivity[] }) {
  const liveStartedAt = useRoundtableStore((s) => s.liveStartedAt);
  const lastActivityAt = useRoundtableStore((s) => s.lastActivityAt);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);

  const elapsed = liveStartedAt ? now - liveStartedAt : 0;
  const quietSince = lastActivityAt ?? liveStartedAt ?? now;
  const stale = now - quietSince > STALE_AFTER_MS;

  const last = live[live.length - 1];
  let action = "starting turn…";
  if (last?.kind === "tool") action = last.text ? `${last.label} — ${last.text}` : last.label;
  else if (last?.kind === "thinking") action = "thinking…";
  else if (last?.kind === "text") action = "writing response…";

  return (
    <span className="rt-livestatus" title={action}>
      <span className="wb-spinner" />
      <span className="rt-livestatus-action">{action}</span>
      <span className={`rt-elapsed ${stale ? "rt-elapsed-stale" : ""}`}>
        {stale ? `quiet ${fmtDur(now - quietSince)}` : fmtDur(elapsed)}
      </span>
    </span>
  );
}

function fmtDur(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
}

function RosterChip({
  id,
  name,
  model,
  engine,
  active,
}: {
  id: string;
  name: string;
  model: string;
  engine?: string;
  active: boolean;
}) {
  const color = authorColor(id);
  return (
    <div
      className={`rt-roster-chip ${active ? "rt-roster-active" : ""}`}
      style={{ borderColor: active ? color : undefined }}
    >
      <span className="rt-dot" style={{ background: color }} />
      <span className="rt-roster-name">{name}</span>
      <span className="rt-roster-model">
        {engine === "codex" ? "◆" : "✶"} {model}
      </span>
      {active && (
        <span className="rt-roster-turn">
          <span className="wb-spinner" /> turn
        </span>
      )}
    </div>
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
          return (
            <div key={i} className="rt-act rt-act-thinking">
              🧠 {a.text}
            </div>
          );
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

function MessageBubble({
  turn,
  activities,
}: {
  turn: RoundtableTurn;
  activities: RoundtableActivity[];
}) {
  const color = authorColor(turn.authorId);
  const steps = activities.filter((a) => a.kind !== "text");

  if (turn.isHuman) {
    return (
      <div className="rt-turn rt-msg-human" style={{ borderLeftColor: color }}>
        <div className="rt-turn-head">
          <span className="rt-dot" style={{ background: color }} />
          <span className="rt-turn-name">{turn.authorName}</span>
        </div>
        <div className="rt-turn-body">
          <MarkdownText content={turn.text} />
        </div>
      </div>
    );
  }

  return (
    <div className="rt-turn" style={{ borderLeftColor: color }}>
      <div className="rt-turn-head">
        <span className="rt-dot" style={{ background: color }} />
        <span className="rt-turn-name">{turn.authorName}</span>
        <span className="rt-turn-model">
          {turn.engine === "codex" ? "◆" : "✶"} {turn.model}
        </span>
        {turn.kind && (
          <span className="rt-turn-kind" title={KIND_HINT[turn.kind] ?? turn.kind}>
            {KIND_LABEL[turn.kind] ?? turn.kind}
          </span>
        )}
        <span className="spacer" />
        <span className="rt-turn-round">t{turn.turn}</span>
      </div>
      {steps.length > 0 && (
        <details className="rt-steps">
          <summary>
            {steps.length} step{steps.length === 1 ? "" : "s"} — what it did
          </summary>
          <ActivityFeed items={steps} showText={false} />
        </details>
      )}
      <div className="rt-turn-body">
        <MarkdownText content={turn.text} />
      </div>
    </div>
  );
}

const KIND_LABEL: Record<string, string> = {
  delegated: "⇢ delegated task",
  return: "⇠ result returned",
  answer: "↳ answer",
  question: "? question",
  kickoff: "▶ kick-off",
  review: "✓ review",
  consult: "💬 consultation",
  correction: "↻ correction",
};
const KIND_HINT: Record<string, string> = {
  delegated: "This turn ran a task a peer delegated through the connector",
  return: "The connector handed this agent the result of a task it delegated",
  answer: "Your answer to the agent's question",
  question: "The agent asked you a question and ended its turn",
  kickoff: "The organizer received the job's objective",
  review: "The reviewer judged the current result",
  consult: "A peer's consultation, answered without changing files",
  correction: "Addressing the reviewer's findings",
};

/// Where a job stands, derived from the connector view and the room phase.
function jobStatus(view: ConnectorView, turns: RoundtableTurn[], phase: string): string {
  if (phase === "done") return "completed";
  const active = view.tasks.find((t) => t.stage !== "delivered" && t.stage !== "cancelled");
  if (active?.kind === "correction") return "correcting";
  if (active) return "implementing";
  const last = turns[turns.length - 1];
  if (last?.kind === "review") return "reviewing";
  if (!turns.some((t) => t.kind === "kickoff")) return "kick-off";
  return "settling";
}

/// What the agents did through the connector: a pending question (answer it
/// here — the room is waiting), the delegations with their stage, and any
/// recorded reviews. Hidden while there is nothing to show.
function ConnectorBlock() {
  const connector = useRoundtableStore((s) => s.connector);
  const roster = useRoundtableStore((s) => s.roster);
  const readOnly = useRoundtableStore((s) => s.readOnly);
  const jobMode = useRoundtableStore((s) => s.jobMode);
  const turns = useRoundtableStore((s) => s.turns);
  const phase = useRoundtableStore((s) => s.phase);
  const reported = useRoundtableStore((s) => s.jobPhase);
  if (!connector) return null;
  const status = reported?.phase ?? jobStatus(connector, turns, phase);
  const { tasks, questions, reviews, pendingJobs } = connector;
  const pending = questions.find((q) => q.status === "waiting");
  const waitingJobs = pendingJobs.filter((j) => j.status === "pending_approval");
  if (
    tasks.length === 0 &&
    questions.length === 0 &&
    reviews.length === 0 &&
    pendingJobs.length === 0 &&
    !jobMode
  )
    return null;
  const name = (id: string) => roster.find((p) => p.id === id)?.name ?? id;
  const corrections = reviews.filter((r) => r.verdict === "changes").length;

  return (
    <div className="rt-connector">
      <div className="rt-connector-head">
        <span>{jobMode ? "job" : "connector"}</span>
        <span className="rt-meta-sep">·</span>
        {jobMode && (
          <>
            <span className={`rt-stage rt-stage-${status}`}>{status}</span>
            {reported && reported.maxCorrections > 0 && (
              <span title="changes verdicts absorbed so far / limit">
                {reported.corrections}/{reported.maxCorrections} corrections
              </span>
            )}
            <span className="rt-meta-sep">·</span>
          </>
        )}
        <span>
          {tasks.length} delegation{tasks.length === 1 ? "" : "s"}
          {reviews.length > 0 &&
            ` · ${reviews.length} review${reviews.length === 1 ? "" : "s"}${
              corrections ? ` (${corrections} changes)` : ""
            }`}
          {waitingJobs.length > 0 && ` · ${waitingJobs.length} awaiting your approval`}
        </span>
      </div>
      {pending && !readOnly && <QuestionCard q={pending} askerName={name(pending.sender)} />}
      {waitingJobs.map((j) => (
        <PendingJobCard key={j.id} job={j} creatorName={name(j.creator)} readOnly={readOnly} />
      ))}
      {tasks.map((t) => (
        <DelegationRow key={t.id} t={t} name={name} />
      ))}
      {reviews.map((r) => (
        <div key={r.id} className="rt-review">
          <span className="rt-delegation-who">{name(r.participant)} reviewed</span>{" "}
          <span className={`rt-stage rt-stage-${r.verdict}`}>{r.verdict}</span>{" "}
          <span className="rt-delegation-who">@{r.revision}</span>
          <div className="rt-review-body">{r.body}</div>
        </div>
      ))}
    </div>
  );
}

/// Work an agent split off with `create_task`: nothing runs until you decide.
/// Approve starts it as a new job room with the same team (and the panel
/// follows it); Discard drops it.
function PendingJobCard({
  job,
  creatorName,
  readOnly,
}: {
  job: ConnectorPendingJob;
  creatorName: string;
  readOnly: boolean;
}) {
  const resolvePending = useRoundtableStore((s) => s.resolvePending);
  return (
    <div className="rt-pending">
      <div className="rt-delegation-who">{creatorName} proposes a follow-up job</div>
      <div className="rt-question-body">{job.instructions}</div>
      {!readOnly && (
        <div className="rt-pending-actions">
          <button
            className="wb-cta wb-cta-sm"
            onClick={() => void resolvePending(job.id, true)}
            title="Start it now as its own job room with the same team; the panel switches to it"
          >
            Approve & start
          </button>
          <button
            className="rt-question-option"
            onClick={() => void resolvePending(job.id, false)}
            title="Drop this proposal"
          >
            Discard
          </button>
        </div>
      )}
    </div>
  );
}

function DelegationRow({ t, name }: { t: ConnectorTask; name: (id: string) => string }) {
  const outcome = t.outcome === "failed" ? "failed" : t.stage;
  const detail =
    t.stage === "delivered" || t.stage === "ready"
      ? (t.error ?? t.result ?? "")
      : t.stage === "delivery_failed"
        ? (t.error ?? "")
        : "";
  return (
    <div className="rt-delegation" title={detail || t.instructions}>
      <span className="rt-delegation-who">
        {name(t.sender)} → {name(t.recipient)}
      </span>
      <span className="rt-delegation-what">{t.instructions}</span>
      <span className={`rt-stage rt-stage-${outcome}`}>
        {t.outcome === "failed" ? "failed" : t.stage.replace("_", " ")}
      </span>
    </div>
  );
}

/// The agent's `ask_user` question. Picking an option answers with its label;
/// free text is the alternative. Either way the room resumes with the asker.
function QuestionCard({ q, askerName }: { q: ConnectorQuestion; askerName: string }) {
  const answerDraft = useRoundtableStore((s) => s.answerDraft);
  const setAnswerDraft = useRoundtableStore((s) => s.setAnswerDraft);
  const answerQuestion = useRoundtableStore((s) => s.answerQuestion);
  const answering = useRoundtableStore((s) => s.answering);
  return (
    <div className="rt-question">
      <div className="rt-delegation-who">{askerName} asks you</div>
      <div className="rt-question-body">{q.body}</div>
      {q.options.length > 0 && (
        <div className="rt-question-options">
          {q.options.map((o) => (
            <button
              key={o.id}
              type="button"
              className="rt-question-option"
              disabled={answering}
              onClick={() => void answerQuestion(q.id, o.id)}
            >
              {o.label}
              {o.description && <span className="rt-question-option-desc">{o.description}</span>}
            </button>
          ))}
        </div>
      )}
      <div className="rt-question-free">
        <input
          placeholder={q.options.length ? "…or write another answer" : "Your answer…"}
          value={answerDraft}
          disabled={answering}
          onChange={(e) => setAnswerDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && answerDraft.trim()) void answerQuestion(q.id, null);
          }}
        />
        <button
          className="wb-cta wb-cta-sm"
          disabled={answering || !answerDraft.trim()}
          onClick={() => void answerQuestion(q.id, null)}
        >
          {answering ? "Sending…" : "Answer"}
        </button>
      </div>
    </div>
  );
}

function HumanInput() {
  const phase = useRoundtableStore((s) => s.phase);
  const injectDraft = useRoundtableStore((s) => s.injectDraft);
  const setInjectDraft = useRoundtableStore((s) => s.setInjectDraft);
  const inject = useRoundtableStore((s) => s.inject);
  const continueRoom = useRoundtableStore((s) => s.continueRoom);

  // Hidden only on a hard end. "awaiting" (turn limit reached) keeps the input
  // so the human can steer and continue the conversation.
  if (phase === "done" || phase === "stopped" || phase === "error") return null;

  const awaiting = phase === "awaiting";
  // When the room is waiting on us, sending a message also restarts it; while
  // it's live, a message just joins the next turn.
  const send = async () => {
    if (!injectDraft.trim()) return;
    await inject();
    if (awaiting) void continueRoom();
  };

  return (
    <div className="rt-moderator">
      <input
        placeholder={
          awaiting
            ? "Add a message and the room continues — or just hit Continue…"
            : "Join the conversation — your message is seen by everyone on their next turn…"
        }
        value={injectDraft}
        onChange={(e) => setInjectDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && injectDraft.trim()) void send();
        }}
      />
      <button
        className="wb-cta wb-cta-sm"
        onClick={() => void send()}
        disabled={!injectDraft.trim()}
      >
        Send
      </button>
      {awaiting && (
        <button className="wb-cta wb-cta-sm rt-continue" onClick={() => void continueRoom()}>
          Continue ▸
        </button>
      )}
    </div>
  );
}

function formatTokens(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return `${n}`;
}
