import { useEffect, useRef, useState } from "react";

import { useRoundtableStore, modelsFor, type RtParticipantDraft } from "../stores/roundtableStore";
import { useChangesStore } from "../stores/changesStore";
import { AGENT_PROFILES } from "../agents/profiles";
import { MarkdownText } from "./MarkdownText";
import type { RoundtableActivity, RoundtableTurn } from "../types/domain";

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

      {message && (
        <p className="wb-hint" style={{ color: "#ff8585" }}>
          {message}
        </p>
      )}

      <button className="wb-cta" onClick={start} disabled={!draft.problem.trim()}>
        Start conversation
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
