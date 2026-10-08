import { useEffect } from "react";

import { useJobsStore } from "../stores/jobsStore";
import { useRoundtableStore } from "../stores/roundtableStore";
import { useSessionStore } from "../stores/sessionStore";
import type { JobCard, JobColumn, JobPendingCard } from "../types/domain";
import { confirmDialog } from "../stores/confirmStore";

/// ai-connector's kanban grouping: proposals first, then the queue.
const COLUMNS: { key: JobColumn | "pending_approval"; label: string; hint: string }[] = [
  {
    key: "pending_approval",
    label: "Awaiting approval",
    hint: "Follow-up work an agent split off with create_task — nothing runs until you approve it",
  },
  { key: "queued", label: "Queued", hint: "Waiting for a free job slot in this project" },
  { key: "running", label: "Running", hint: "Holding a slot and working" },
  {
    key: "needs_attention",
    label: "Needs attention",
    hint: "Paused, waiting for you (a question, a blocked review, a landing to confirm) or interrupted",
  },
  { key: "completed", label: "Completed", hint: "Approved and, for working rooms, landed" },
  { key: "closed", label: "Closed", hint: "Stopped or closed by you" },
];

function relTime(ms: number): string {
  const d = Math.max(0, Date.now() - ms);
  if (d < 60_000) return "just now";
  if (d < 3_600_000) return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)}h ago`;
  return `${Math.floor(d / 86_400_000)}d ago`;
}

/// Switch the workbench to the Room tab (the panel strip listens to this
/// event, the same way the command palette navigates).
function openRoomTab() {
  window.dispatchEvent(new CustomEvent("ac:open-workbench-tab", { detail: "roundtable" }));
}

export function JobsPanel() {
  const board = useJobsStore((s) => s.board);
  const error = useJobsStore((s) => s.error);
  const load = useJobsStore((s) => s.load);
  const initListeners = useJobsStore((s) => s.initListeners);
  const setParallel = useJobsStore((s) => s.setParallel);
  const projectRoot = useSessionStore((s) => s.project?.root);
  const setDraft = useRoundtableStore((s) => s.setDraft);
  const reset = useRoundtableStore((s) => s.reset);

  useEffect(() => {
    void initListeners();
    void load();
  }, [initListeners, load, projectRoot]);

  const newJob = async () => {
    // Fresh config form in job mode; the room panel does the rest.
    await reset();
    setDraft({ jobMode: true });
    openRoomTab();
  };

  if (!board) {
    return (
      <section className="wb-section">
        <p className="wb-hint">{error ?? "Loading the jobs board…"}</p>
      </section>
    );
  }

  const byColumn = (key: string) => board.cards.filter((c) => c.column === key);

  return (
    <section className="wb-section jobs-panel">
      <div className="jobs-head">
        <p className="wb-hint">
          Every job room of this project, by state. The organizer drives each job through the
          connector; the queue runs them {board.settings.parallelJobs === 1 ? "one" : "a few"} at a
          time and lands working rooms when you confirm.
        </p>
        <div className="jobs-controls">
          <label className="rt-field rt-field-sm" title="Jobs of this project that may run at once">
            <span>parallel jobs</span>
            <input
              type="number"
              min={1}
              max={8}
              value={board.settings.parallelJobs}
              onChange={(e) => void setParallel(Number(e.target.value))}
            />
          </label>
          <span className="jobs-busy" title="jobs holding a slot right now">
            {board.busy}/{board.settings.parallelJobs} slots
          </span>
          <button className="wb-cta wb-cta-sm" onClick={() => void newJob()}>
            + New job
          </button>
        </div>
      </div>
      {error && <div className="rt-banner rt-banner-error">{error}</div>}
      <div className="jobs-board">
        {COLUMNS.map((col) => {
          const cards = col.key === "pending_approval" ? [] : byColumn(col.key);
          const pending = col.key === "pending_approval" ? board.pending : [];
          const count = cards.length + pending.length;
          return (
            <div key={col.key} className={`jobs-column jobs-column-${col.key}`} title={col.hint}>
              <div className="jobs-column-head">
                <span>{col.label}</span>
                <span className="jobs-column-count">{count}</span>
              </div>
              <div className="jobs-column-body">
                {pending.map((p) => (
                  <PendingCardView key={p.pending.id} item={p} />
                ))}
                {cards.map((c) => (
                  <JobCardView key={c.id} card={c} />
                ))}
                {count === 0 && <div className="jobs-empty">—</div>}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}

function PendingCardView({ item }: { item: JobPendingCard }) {
  const resolvePending = useJobsStore((s) => s.resolvePending);
  const busyId = useJobsStore((s) => s.busyId);
  const busy = busyId === item.pending.id;
  return (
    <div className="jobs-card jobs-card-pending" title={item.pending.instructions}>
      <div className="jobs-card-title">{item.pending.instructions}</div>
      <div className="jobs-card-meta">proposed in "{item.sourceProblem.slice(0, 60)}"</div>
      <div className="jobs-card-actions">
        <button
          className="wb-cta wb-cta-sm"
          disabled={busy}
          onClick={() => void resolvePending(item.pending.sourceJobId, item.pending.id, true)}
          title="Start it as its own job room with the same team"
        >
          Approve & start
        </button>
        <button
          className="jobs-btn"
          disabled={busy}
          onClick={() => void resolvePending(item.pending.sourceJobId, item.pending.id, false)}
        >
          Discard
        </button>
      </div>
    </div>
  );
}

function JobCardView({ card }: { card: JobCard }) {
  const startNow = useJobsStore((s) => s.startNow);
  const continueJob = useJobsStore((s) => s.continueJob);
  const closeJob = useJobsStore((s) => s.closeJob);
  const move = useJobsStore((s) => s.move);
  const confirmLanding = useJobsStore((s) => s.confirmLanding);
  const busyId = useJobsStore((s) => s.busyId);
  const attachRoom = useRoundtableStore((s) => s.attachRoom);
  const busy = busyId === card.id;

  const open = async () => {
    await attachRoom(card.id, card.status);
    openRoomTab();
  };
  const close = async () => {
    if (
      await confirmDialog({
        title: "Close job",
        message: `Close "${card.problem.slice(0, 60)}"? A running turn is stopped; the room and its branch stay inspectable.`,
        confirmLabel: "Close job",
        danger: true,
      })
    ) {
      void closeJob(card.id);
    }
  };

  return (
    <div className={`jobs-card jobs-card-${card.status}`} title={card.reason ?? card.problem}>
      <div className="jobs-card-title" onClick={() => void open()}>
        {card.originRoomId ? "↳ " : ""}
        {card.problem}
      </div>
      <div className="jobs-card-meta">
        <span className={`rt-stage rt-stage-${card.status}`}>{card.status.replace("_", " ")}</span>
        {card.phase && <span className="jobs-phase">{card.phase}</span>}
        {card.allowEdits && <span title="working room (edits a room/… branch)">✎</span>}
        <span className="spacer" />
        <span>
          {card.lastTurn}t · {relTime(card.updatedAtMs)}
        </span>
      </div>
      <div className="jobs-card-team">{card.participantNames.join(" · ")}</div>
      {card.reason && <div className="jobs-card-reason">{card.reason}</div>}
      <div className="jobs-card-actions">
        <button className="jobs-btn" onClick={() => void open()} title="Open the room">
          Open
        </button>
        {card.status === "queued" && (
          <>
            <button
              className="wb-cta wb-cta-sm"
              disabled={busy}
              onClick={() => void startNow(card.id)}
              title="Run it now, ahead of the slot limit"
            >
              Start now
            </button>
            <button
              className="jobs-btn"
              disabled={busy}
              onClick={() => void move(card.id, true)}
              title="Earlier in the queue"
            >
              ▲
            </button>
            <button
              className="jobs-btn"
              disabled={busy}
              onClick={() => void move(card.id, false)}
              title="Later in the queue"
            >
              ▼
            </button>
          </>
        )}
        {card.status === "awaiting_confirmation" && (
          <button
            className="wb-cta wb-cta-sm"
            disabled={busy}
            onClick={() => void confirmLanding(card.id)}
            title="Fast-forward the base branch to the merged, reviewed job branch and clean up"
          >
            Land ▸
          </button>
        )}
        {(card.status === "needs_attention" || card.status === "paused") && (
          <button
            className="wb-cta wb-cta-sm"
            disabled={busy}
            onClick={() => void continueJob(card.id)}
            title="Run more turns (restores the room if it was only saved)"
          >
            Continue ▸
          </button>
        )}
        {card.status !== "completed" && card.status !== "closed" && (
          <button className="jobs-btn jobs-btn-danger" disabled={busy} onClick={() => void close()}>
            Close
          </button>
        )}
      </div>
    </div>
  );
}
