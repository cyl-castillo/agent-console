import { useEffect, useMemo, useState } from "react";

import { SCHEDULER_EVENTS, useSchedulerStore } from "../stores/schedulerStore";
import { useSkillsStore } from "../stores/skillsStore";
import type {
  Action,
  Job,
  OnMissed,
  PipelineStep,
  RunRecord,
  StepCondition,
  Trigger,
} from "../types/domain";

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

// ---- time helpers: jobs store daily/weekly times in UTC; the UI edits local --

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

function localTimeToUtc(hhmm: string): { hour: number; minute: number } {
  const [h, m] = hhmm.split(":").map(Number);
  const d = new Date();
  d.setHours(h || 0, m || 0, 0, 0);
  return { hour: d.getUTCHours(), minute: d.getUTCMinutes() };
}

function utcTimeToLocal(hour: number, minute: number): string {
  const d = new Date();
  d.setUTCHours(hour, minute, 0, 0);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function localWeeklyToUtc(
  weekday: number,
  hhmm: string,
): { weekday: number; hour: number; minute: number } {
  const [h, m] = hhmm.split(":").map(Number);
  const d = new Date();
  d.setDate(d.getDate() + (weekday - d.getDay()));
  d.setHours(h || 0, m || 0, 0, 0);
  return { weekday: d.getUTCDay(), hour: d.getUTCHours(), minute: d.getUTCMinutes() };
}

function utcWeeklyToLocal(
  weekday: number,
  hour: number,
  minute: number,
): { weekday: number; time: string } {
  const d = new Date();
  d.setUTCDate(d.getUTCDate() + (weekday - d.getUTCDay()));
  d.setUTCHours(hour, minute, 0, 0);
  return { weekday: d.getDay(), time: `${pad2(d.getHours())}:${pad2(d.getMinutes())}` };
}

// ---- human-readable summaries -------------------------------------------

function humanDuration(ms: number): string {
  if (ms % DAY === 0) return `${ms / DAY} day${ms / DAY === 1 ? "" : "s"}`;
  if (ms % HOUR === 0) return `${ms / HOUR} hour${ms / HOUR === 1 ? "" : "s"}`;
  const mins = Math.max(1, Math.round(ms / MIN));
  return `${mins} min`;
}

function describeTrigger(t: Trigger): string {
  switch (t.type) {
    case "interval":
      return `every ${humanDuration(t.everyMs)}`;
    case "daily":
      return `daily at ${utcTimeToLocal(t.hour, t.minute)}`;
    case "weekly": {
      const l = utcWeeklyToLocal(t.weekday, t.hour, t.minute);
      return `${WEEKDAYS[l.weekday]} at ${l.time}`;
    }
    case "event":
      return `on event: ${t.name || "?"}`;
  }
}

function describeAction(a: Action): string {
  switch (a.type) {
    case "skill":
      return `/${a.name}${a.args ? ` ${a.args}` : ""}`;
    case "prompt":
      return a.text.length > 60 ? `${a.text.slice(0, 60)}…` : a.text;
    case "pipeline":
      return `pipeline · ${a.steps.length} step${a.steps.length === 1 ? "" : "s"}`;
  }
}

function formatWhen(ms?: number): string {
  if (!ms) return "—";
  const d = new Date(ms);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const time = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  return sameDay ? time : `${d.getMonth() + 1}/${d.getDate()} ${time}`;
}

// ---- draft model (local-friendly form state) ----------------------------

interface Draft {
  id: string;
  name: string;
  onMissed: OnMissed;
  cooldownMin: number;
  triggerType: Trigger["type"];
  intervalValue: number;
  intervalUnit: "minutes" | "hours" | "days";
  dailyTime: string;
  weeklyDay: number;
  weeklyTime: string;
  eventName: string;
  actionType: Action["type"];
  skillName: string;
  skillArgs: string;
  promptText: string;
  steps: PipelineStep[];
}

function blankDraft(): Draft {
  return {
    id: "",
    name: "",
    onMissed: "catchup",
    cooldownMin: 0,
    triggerType: "daily",
    intervalValue: 1,
    intervalUnit: "hours",
    dailyTime: "09:00",
    weeklyDay: 1,
    weeklyTime: "09:00",
    eventName: SCHEDULER_EVENTS[0].name,
    actionType: "prompt",
    skillName: "",
    skillArgs: "",
    promptText: "",
    steps: [],
  };
}

function draftFromJob(job: Job): Draft {
  const d = blankDraft();
  d.id = job.id;
  d.name = job.name;
  d.onMissed = job.onMissed;
  d.cooldownMin = Math.round((job.cooldownMs || 0) / MIN);
  d.triggerType = job.trigger.type;
  if (job.trigger.type === "interval") {
    const ms = job.trigger.everyMs;
    if (ms % DAY === 0) {
      d.intervalUnit = "days";
      d.intervalValue = ms / DAY;
    } else if (ms % HOUR === 0) {
      d.intervalUnit = "hours";
      d.intervalValue = ms / HOUR;
    } else {
      d.intervalUnit = "minutes";
      d.intervalValue = Math.max(1, Math.round(ms / MIN));
    }
  } else if (job.trigger.type === "daily") {
    d.dailyTime = utcTimeToLocal(job.trigger.hour, job.trigger.minute);
  } else if (job.trigger.type === "weekly") {
    const l = utcWeeklyToLocal(job.trigger.weekday, job.trigger.hour, job.trigger.minute);
    d.weeklyDay = l.weekday;
    d.weeklyTime = l.time;
  } else {
    d.eventName = job.trigger.name;
  }
  d.actionType = job.action.type;
  if (job.action.type === "skill") {
    d.skillName = job.action.name;
    d.skillArgs = job.action.args ?? "";
  } else if (job.action.type === "prompt") {
    d.promptText = job.action.text;
  } else {
    d.steps = job.action.steps;
  }
  return d;
}

function buildTrigger(d: Draft): Trigger {
  switch (d.triggerType) {
    case "interval": {
      const unitMs = d.intervalUnit === "days" ? DAY : d.intervalUnit === "hours" ? HOUR : MIN;
      return { type: "interval", everyMs: Math.max(1, d.intervalValue) * unitMs };
    }
    case "daily": {
      const u = localTimeToUtc(d.dailyTime);
      return { type: "daily", hour: u.hour, minute: u.minute };
    }
    case "weekly": {
      const u = localWeeklyToUtc(d.weeklyDay, d.weeklyTime);
      return { type: "weekly", weekday: u.weekday, hour: u.hour, minute: u.minute };
    }
    case "event":
      return { type: "event", name: d.eventName.trim() };
  }
}

function buildAction(d: Draft): Action {
  switch (d.actionType) {
    case "skill":
      return {
        type: "skill",
        name: d.skillName.replace(/^\//, "").trim(),
        args: d.skillArgs.trim() || undefined,
      };
    case "prompt":
      return { type: "prompt", text: d.promptText.trim() };
    case "pipeline":
      return { type: "pipeline", steps: d.steps };
  }
}

function validate(d: Draft): string | null {
  if (!d.name.trim()) return "Give the job a name.";
  if (d.triggerType === "interval" && d.intervalValue < 1) return "Interval must be ≥ 1.";
  if (d.triggerType === "event" && !d.eventName.trim()) return "Pick an event name.";
  if (d.actionType === "skill" && !d.skillName.trim()) return "Choose a skill.";
  if (d.actionType === "prompt" && !d.promptText.trim()) return "Write the prompt.";
  if (d.actionType === "pipeline" && d.steps.length === 0) return "Add at least one step.";
  return null;
}

// ==========================================================================

export function SchedulerPanel() {
  const jobs = useSchedulerStore((s) => s.jobs);
  const history = useSchedulerStore((s) => s.history);
  const status = useSchedulerStore((s) => s.status);
  const errorMessage = useSchedulerStore((s) => s.errorMessage);
  const runningJobIds = useSchedulerStore((s) => s.runningJobIds);
  const refresh = useSchedulerStore((s) => s.refresh);
  const createJob = useSchedulerStore((s) => s.createJob);
  const updateJob = useSchedulerStore((s) => s.updateJob);
  const deleteJob = useSchedulerStore((s) => s.deleteJob);
  const setEnabled = useSchedulerStore((s) => s.setEnabled);
  const runNow = useSchedulerStore((s) => s.runNow);

  const [draft, setDraft] = useState<Draft | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (status === "idle") void refresh();
  }, [status, refresh]);

  const startNew = () => {
    setFormError(null);
    setDraft(blankDraft());
  };
  const startEdit = (job: Job) => {
    setFormError(null);
    setDraft(draftFromJob(job));
  };
  const cancelEdit = () => {
    setDraft(null);
    setFormError(null);
  };

  const save = async () => {
    if (!draft) return;
    const err = validate(draft);
    if (err) {
      setFormError(err);
      return;
    }
    const job: Job = {
      id: draft.id,
      name: draft.name.trim(),
      enabled: true,
      trigger: buildTrigger(draft),
      action: buildAction(draft),
      onMissed: draft.onMissed,
      cooldownMs: Math.max(0, draft.cooldownMin) * MIN,
      createdAtMs: 0,
    };
    setSaving(true);
    try {
      if (draft.id) await updateJob(job);
      else await createJob(job);
      setDraft(null);
      setFormError(null);
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">schedule</span>
        <span className="spacer" />
        {!draft && (
          <button className="workbench-action" onClick={startNew} title="New scheduled job">
            + new
          </button>
        )}
        <button
          className="workbench-action"
          onClick={() => void refresh()}
          disabled={status === "loading"}
          title="Refresh"
        >
          {status === "loading" ? "…" : "↻"}
        </button>
      </div>

      <div className="workbench-body">
        {status === "error" && (
          <section className="wb-section">
            <div className="wb-section-title">scheduler error</div>
            <p className="wb-hint" style={{ whiteSpace: "pre-wrap" }}>{errorMessage}</p>
          </section>
        )}

        {draft ? (
          <JobEditor
            draft={draft}
            setDraft={setDraft}
            onSave={save}
            onCancel={cancelEdit}
            saving={saving}
            error={formError}
          />
        ) : (
          <>
            {jobs.length === 0 && status !== "loading" && (
              <section className="wb-section">
                <p className="wb-hint">
                  Schedule a skill, a prompt, or a small pipeline to run on a
                  clock — a nightly digest, a weekly corpus tidy, an hourly
                  status check. Every run goes through Claude in <em>plan mode</em>,
                  so it can only <strong>suggest</strong> — nothing is changed
                  until you act on the result.
                </p>
                <button className="wb-cta" onClick={startNew}>Schedule a job</button>
              </section>
            )}

            {jobs.length > 0 && (
              <section className="wb-section">
                <div className="wb-section-title">
                  jobs<span className="wb-count">{jobs.length}</span>
                </div>
                <div className="wb-job-list">
                  {jobs.map((job) => (
                    <JobCard
                      key={job.id}
                      job={job}
                      running={runningJobIds.includes(job.id)}
                      onRun={() => void runNow(job.id)}
                      onToggle={() => void setEnabled(job.id, !job.enabled)}
                      onEdit={() => startEdit(job)}
                      onDelete={() => void deleteJob(job.id)}
                    />
                  ))}
                </div>
              </section>
            )}

            {history.length > 0 && (
              <section className="wb-section">
                <div className="wb-section-title">
                  recent runs<span className="wb-count">{history.length}</span>
                </div>
                <div className="wb-run-feed">
                  {history.slice(0, 30).map((rec, i) => (
                    <RunRow key={`${rec.jobId}-${rec.startedMs}-${i}`} rec={rec} />
                  ))}
                </div>
              </section>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function JobCard({
  job,
  running,
  onRun,
  onToggle,
  onEdit,
  onDelete,
}: {
  job: Job;
  running: boolean;
  onRun: () => void;
  onToggle: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={`wb-job-card ${job.enabled ? "" : "paused"}`}>
      <div className="wb-job-head">
        <span className="wb-job-name" title={job.name}>{job.name}</span>
        {running && <span className="wb-job-running" title="Running now">running…</span>}
        {!job.enabled && !running && <span className="wb-job-paused-tag">paused</span>}
      </div>
      <div className="wb-job-meta">
        <span className="wb-job-trigger">{describeTrigger(job.trigger)}</span>
        <span className="wb-job-sep">·</span>
        <span className="wb-job-action" title={describeAction(job.action)}>
          {describeAction(job.action)}
        </span>
      </div>
      <div className="wb-job-times">
        <span>next: {job.enabled ? formatWhen(job.nextDueMs) : "—"}</span>
        <span>last: {formatWhen(job.lastRunMs)}</span>
      </div>
      <div className="wb-job-actions">
        <button
          className="wb-job-btn"
          onClick={onRun}
          disabled={running}
          title="Run now"
        >
          ▶ run
        </button>
        <button className="wb-job-btn" onClick={onToggle} title={job.enabled ? "Pause" : "Resume"}>
          {job.enabled ? "⏸ pause" : "▶ resume"}
        </button>
        <button className="wb-job-btn" onClick={onEdit} title="Edit">edit</button>
        <button className="wb-job-btn danger" onClick={onDelete} title="Delete">✕</button>
      </div>
    </div>
  );
}

function RunRow({ rec }: { rec: RunRecord }) {
  const cls =
    rec.status === "ok" ? "ok" : rec.status === "missed" ? "missed" : "error";
  return (
    <div className={`wb-run-row ${cls}`}>
      <div className="wb-run-head">
        <span className={`wb-run-status ${cls}`}>{rec.status}</span>
        <span className="wb-run-name">{rec.jobName}</span>
        <span className="wb-run-when">{formatWhen(rec.startedMs)}</span>
      </div>
      {rec.summary && <div className="wb-run-summary">{rec.summary}</div>}
    </div>
  );
}

function JobEditor({
  draft,
  setDraft,
  onSave,
  onCancel,
  saving,
  error,
}: {
  draft: Draft;
  setDraft: (d: Draft) => void;
  onSave: () => void;
  onCancel: () => void;
  saving: boolean;
  error: string | null;
}) {
  const skills = useSkillsStore((s) => s.installed);
  const skillNames = useMemo(
    () =>
      skills
        .filter((sk) => sk.kind === "skill" || sk.kind === "command")
        .map((sk) => sk.name)
        .filter((n, i, a) => a.indexOf(n) === i),
    [skills],
  );
  const set = (patch: Partial<Draft>) => setDraft({ ...draft, ...patch });

  return (
    <section className="wb-section wb-job-editor">
      <div className="wb-section-title">{draft.id ? "edit job" : "new job"}</div>

      <label className="wb-field">
        <span className="wb-field-label">name</span>
        <input
          className="wb-input"
          value={draft.name}
          placeholder="Nightly digest"
          onChange={(e) => set({ name: e.target.value })}
        />
      </label>

      {/* trigger */}
      <label className="wb-field">
        <span className="wb-field-label">when</span>
        <select
          className="wb-input"
          value={draft.triggerType}
          onChange={(e) => set({ triggerType: e.target.value as Trigger["type"] })}
        >
          <option value="interval">every…</option>
          <option value="daily">daily at…</option>
          <option value="weekly">weekly on…</option>
          <option value="event">on event…</option>
        </select>
      </label>

      {draft.triggerType === "interval" && (
        <div className="wb-field-row">
          <input
            className="wb-input wb-input-narrow"
            type="number"
            min={1}
            value={draft.intervalValue}
            onChange={(e) => set({ intervalValue: Number(e.target.value) })}
          />
          <select
            className="wb-input"
            value={draft.intervalUnit}
            onChange={(e) => set({ intervalUnit: e.target.value as Draft["intervalUnit"] })}
          >
            <option value="minutes">minutes</option>
            <option value="hours">hours</option>
            <option value="days">days</option>
          </select>
        </div>
      )}

      {draft.triggerType === "daily" && (
        <input
          className="wb-input"
          type="time"
          value={draft.dailyTime}
          onChange={(e) => set({ dailyTime: e.target.value })}
        />
      )}

      {draft.triggerType === "weekly" && (
        <div className="wb-field-row">
          <select
            className="wb-input"
            value={draft.weeklyDay}
            onChange={(e) => set({ weeklyDay: Number(e.target.value) })}
          >
            {WEEKDAYS.map((d, i) => (
              <option key={i} value={i}>{d}</option>
            ))}
          </select>
          <input
            className="wb-input"
            type="time"
            value={draft.weeklyTime}
            onChange={(e) => set({ weeklyTime: e.target.value })}
          />
        </div>
      )}

      {draft.triggerType === "event" && (
        <>
          <select
            className="wb-input"
            value={draft.eventName}
            onChange={(e) => set({ eventName: e.target.value })}
          >
            {SCHEDULER_EVENTS.map((ev) => (
              <option key={ev.name} value={ev.name}>{ev.label}</option>
            ))}
          </select>
          <p className="wb-hint wb-hint-sm">
            Fires whenever this happens in the app. Set a cooldown below so a
            burst (many prompts, several commits) doesn’t run it repeatedly.
          </p>
          <label className="wb-field" style={{ marginTop: 8 }}>
            <span className="wb-field-label">cooldown (min)</span>
            <input
              className="wb-input wb-input-narrow"
              type="number"
              min={0}
              value={draft.cooldownMin}
              onChange={(e) => set({ cooldownMin: Number(e.target.value) })}
            />
          </label>
        </>
      )}

      {/* action */}
      <label className="wb-field">
        <span className="wb-field-label">do</span>
        <select
          className="wb-input"
          value={draft.actionType}
          onChange={(e) => set({ actionType: e.target.value as Action["type"] })}
        >
          <option value="prompt">run a prompt</option>
          <option value="skill">run a skill</option>
          <option value="pipeline">run a pipeline</option>
        </select>
      </label>

      {draft.actionType === "skill" && (
        <ActionSkillFields
          name={draft.skillName}
          args={draft.skillArgs}
          skillNames={skillNames}
          onName={(skillName) => set({ skillName })}
          onArgs={(skillArgs) => set({ skillArgs })}
        />
      )}

      {draft.actionType === "prompt" && (
        <textarea
          className="wb-input wb-textarea"
          rows={4}
          value={draft.promptText}
          placeholder="Summarize what changed in this repo today and flag anything risky."
          onChange={(e) => set({ promptText: e.target.value })}
        />
      )}

      {draft.actionType === "pipeline" && (
        <PipelineEditor
          steps={draft.steps}
          skillNames={skillNames}
          onChange={(steps) => set({ steps })}
        />
      )}

      {/* missed policy */}
      <label className="wb-field">
        <span className="wb-field-label">if missed</span>
        <select
          className="wb-input"
          value={draft.onMissed}
          onChange={(e) => set({ onMissed: e.target.value as OnMissed })}
        >
          <option value="catchup">run once on next launch</option>
          <option value="skip">skip the missed run</option>
        </select>
      </label>

      {error && <p className="wb-form-error">{error}</p>}

      <div className="wb-job-editor-actions">
        <button className="wb-cta" onClick={onSave} disabled={saving}>
          {saving ? "saving…" : draft.id ? "save" : "create"}
        </button>
        <button className="wb-job-btn" onClick={onCancel} disabled={saving}>cancel</button>
      </div>
    </section>
  );
}

function ActionSkillFields({
  name,
  args,
  skillNames,
  onName,
  onArgs,
}: {
  name: string;
  args: string;
  skillNames: string[];
  onName: (v: string) => void;
  onArgs: (v: string) => void;
}) {
  return (
    <>
      <input
        className="wb-input"
        list="wb-skill-list"
        value={name}
        placeholder="skill name (e.g. reflect)"
        onChange={(e) => onName(e.target.value)}
      />
      <datalist id="wb-skill-list">
        {skillNames.map((n) => (
          <option key={n} value={n} />
        ))}
      </datalist>
      <input
        className="wb-input"
        value={args}
        placeholder="optional args"
        onChange={(e) => onArgs(e.target.value)}
      />
    </>
  );
}

// Condition <-> select-value helpers (the select can't hold the "contains" text).
type CondKind = "always" | "prevOk" | "prevFailed" | "contains";

function condKind(when?: StepCondition): CondKind {
  if (!when) return "always";
  return when.type;
}

function buildCondition(kind: CondKind, text: string): StepCondition | undefined {
  switch (kind) {
    case "always":
      return undefined;
    case "prevOk":
      return { type: "prevOk" };
    case "prevFailed":
      return { type: "prevFailed" };
    case "contains":
      return { type: "contains", text };
  }
}

function PipelineEditor({
  steps,
  skillNames,
  onChange,
}: {
  steps: PipelineStep[];
  skillNames: string[];
  onChange: (steps: PipelineStep[]) => void;
}) {
  const addSkill = () =>
    onChange([...steps, { action: { type: "skill", name: "" } }]);
  const addPrompt = () =>
    onChange([...steps, { action: { type: "prompt", text: "" } }]);
  const update = (i: number, step: PipelineStep) =>
    onChange(steps.map((s, j) => (j === i ? step : s)));
  const remove = (i: number) => onChange(steps.filter((_, j) => j !== i));

  return (
    <div className="wb-pipeline">
      {steps.map((step, i) => {
        const kind = condKind(step.when);
        const containsText = step.when?.type === "contains" ? step.when.text : "";
        return (
          <div className="wb-pipeline-step-box" key={i}>
            <div className="wb-pipeline-step">
              <span className="wb-pipeline-num">{i + 1}</span>
              {step.action.type === "skill" ? (
                <input
                  className="wb-input"
                  list="wb-skill-list"
                  value={step.action.name}
                  placeholder="skill name"
                  onChange={(e) =>
                    update(i, { ...step, action: { type: "skill", name: e.target.value } })
                  }
                />
              ) : step.action.type === "prompt" ? (
                <input
                  className="wb-input"
                  value={step.action.text}
                  placeholder="prompt"
                  onChange={(e) =>
                    update(i, { ...step, action: { type: "prompt", text: e.target.value } })
                  }
                />
              ) : (
                <span className="wb-hint wb-hint-sm">nested pipeline</span>
              )}
              <button className="wb-job-btn danger" onClick={() => remove(i)} title="Remove step">✕</button>
            </div>
            {/* The first step always runs; later steps can branch on the prior. */}
            {i > 0 && (
              <div className="wb-pipeline-cond">
                <span className="wb-pipeline-cond-label">run</span>
                <select
                  className="wb-input"
                  value={kind}
                  onChange={(e) =>
                    update(i, {
                      ...step,
                      when: buildCondition(e.target.value as CondKind, containsText),
                    })
                  }
                >
                  <option value="always">if previous succeeded (default)</option>
                  <option value="prevFailed">if previous failed</option>
                  <option value="contains">if previous output contains…</option>
                </select>
                {kind === "contains" && (
                  <input
                    className="wb-input"
                    value={containsText}
                    placeholder="e.g. anomaly"
                    onChange={(e) =>
                      update(i, { ...step, when: { type: "contains", text: e.target.value } })
                    }
                  />
                )}
              </div>
            )}
          </div>
        );
      })}
      <datalist id="wb-skill-list">
        {skillNames.map((n) => (
          <option key={n} value={n} />
        ))}
      </datalist>
      <div className="wb-pipeline-add">
        <button className="wb-job-btn" onClick={addSkill}>+ skill step</button>
        <button className="wb-job-btn" onClick={addPrompt}>+ prompt step</button>
      </div>
    </div>
  );
}
