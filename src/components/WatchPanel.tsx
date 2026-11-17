import { useEffect, useState, type FormEvent } from "react";

import { confirmDialog } from "../stores/confirmStore";
import { useProofStore } from "../stores/proofStore";
import { useSessionStore } from "../stores/sessionStore";
import { useShellStore } from "../stores/shellStore";
import { useToastStore } from "../stores/toastStore";
import { useWatchStore } from "../stores/watchStore";
import type { ProofEvent, WatchAlert, WatchExchange, WatchGoal } from "../types/domain";
import { Button, Chip, EmptyState, Field, Input, SectionHeader } from "../ui";
import { cn } from "@/lib/cn";

const time = (ms: number) =>
  new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
const shortSha = (sha: string) => sha.slice(0, 8);
const str = (v: unknown) => (typeof v === "string" ? v : "");
const fail = (err: unknown) =>
  useToastStore
    .getState()
    .show(`Watch: ${err instanceof Error ? err.message : String(err)}`, "error");

/// Goal watch (experimental): state a goal, follow the runs the ledger
/// witnesses, get one alert when a run fails a check, open its evidence.
/// All rules and state live in Rust (`watch_service`); this is a view.
export function WatchPanel() {
  const project = useSessionStore((s) => s.project?.root ?? null);
  const goals = useWatchStore((s) => s.goals);
  const source = useWatchStore((s) => s.source);
  const status = useWatchStore((s) => s.status);
  const errorMessage = useWatchStore((s) => s.errorMessage);
  const refresh = useWatchStore((s) => s.refresh);
  const start = useWatchStore((s) => s.start);
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (project) void refresh();
  }, [project, refresh]);

  const begin = async (demo: boolean) => {
    const text = title.trim() || (demo ? "Demo: close PR #142 today" : "");
    if (!text) return;
    setBusy(true);
    try {
      await start(text, demo);
      setTitle("");
    } catch (err) {
      fail(err);
    } finally {
      setBusy(false);
    }
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    void begin(false);
  };

  return (
    <div className="flex h-full min-h-0 flex-col font-sans text-body" data-testid="watch-panel">
      <div className="min-h-0 flex-1 overflow-auto">
        <form onSubmit={submit} className="flex flex-col gap-2 border-b border-border p-3">
          <p className="m-0 flex items-center gap-2 text-caption text-muted-foreground">
            <Chip tone="purple">experimental</Chip>
            Reads the ledger of this project. It never writes to it.
          </p>
          <Field
            label="What are you trying to get done?"
            description="One condition for now: a run fails a recognized test or check. You get one alert per run, with the ledger lines behind it."
          >
            <Input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Close this PR today"
              maxLength={200}
              data-testid="watch-title"
            />
          </Field>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="submit"
              variant="primary"
              disabled={!title.trim()}
              loading={busy}
              data-testid="watch-start"
            >
              Start watching
            </Button>
            <Button onClick={() => void begin(true)} disabled={busy} data-testid="watch-demo">
              Start demo
            </Button>
            <span className="text-caption text-muted-foreground">
              The demo replays a scripted run that fails after 30 s. It never touches your ledger.
            </span>
          </div>
          {source && !source.witness && (
            <p className="m-0 text-small text-[var(--warn-ink)]" role="status">
              Witnessing is off for this project, so no new run reaches the ledger and there is
              nothing to follow. Switch it on in Proof, or use the demo.
            </p>
          )}
        </form>

        {status === "error" && (
          <p className="m-0 p-3 text-small text-danger" role="alert">
            Could not load the watch: {errorMessage}
          </p>
        )}
        {status === "ready" && goals.length === 0 && (
          <EmptyState
            title="No goal yet"
            description="State what you want to get done and the console follows the runs in this project from now on. Start the demo to see an alert without running an agent."
          />
        )}
        {goals.map((g) => (
          <GoalCard key={g.id} goal={g} />
        ))}
      </div>
    </div>
  );
}

function statusTone(status: string): "ok" | "warn" | "neutral" {
  return status === "active" ? "ok" : status === "paused" ? "warn" : "neutral";
}

function GoalCard({ goal }: { goal: WatchGoal }) {
  const setStatus = useWatchStore((s) => s.setStatus);
  const remove = useWatchStore((s) => s.remove);
  const focus = useWatchStore((s) => s.focus);
  const alerts = goal.alerts.filter((a) => a.state !== "dismissed").reverse();
  const dismissed = goal.alerts.length - alerts.length;

  const act = (fn: () => Promise<void>) => void fn().catch(fail);
  const stop = async () => {
    const ok = await confirmDialog({
      title: "Stop watching this goal?",
      message:
        "A stopped goal keeps its alerts and its thread for reading and is never evaluated again. To follow it later, start a new goal.",
      confirmLabel: "Stop",
    });
    if (ok) await setStatus(goal.id, "stopped");
  };
  const del = async () => {
    const ok = await confirmDialog({
      title: "Delete this goal?",
      message:
        "The goal, its alerts and its thread are removed. The ledger lines they point to are not touched.",
      confirmLabel: "Delete",
      danger: true,
    });
    if (ok) await remove(goal.id);
  };

  return (
    <section className="border-b border-border" data-testid="watch-goal" aria-label={goal.title}>
      <div className="flex flex-wrap items-center gap-2 px-3 pt-3">
        <h3 className="m-0 min-w-0 flex-1 truncate text-body font-semibold text-foreground">
          {goal.title}
        </h3>
        <Chip tone={statusTone(goal.status)} data-testid="watch-goal-status">
          {goal.status}
        </Chip>
        {goal.source === "demo" && <Chip tone="purple">demo</Chip>}
      </div>
      <p className="m-0 px-3 pt-1 text-caption text-muted-foreground">
        Since {time(goal.createdMs)} · alerts when a run fails a check ·{" "}
        {goal.cursorSeq == null
          ? "no events evaluated yet"
          : `evaluated up to seq ${goal.cursorSeq}`}
      </p>
      <div className="flex flex-wrap gap-1.5 px-3 py-2">
        {goal.status === "active" && (
          <Button size="sm" onClick={() => act(() => setStatus(goal.id, "paused"))}>
            Pause
          </Button>
        )}
        {goal.status === "paused" && (
          <Button size="sm" onClick={() => act(() => setStatus(goal.id, "active"))}>
            Resume
          </Button>
        )}
        {goal.status !== "stopped" && (
          <Button size="sm" onClick={() => act(stop)}>
            Stop
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={() => act(del)}>
          Delete
        </Button>
      </div>

      <SectionHeader title="Alerts" count={alerts.length} />
      {alerts.length === 0 ? (
        <p className="m-0 px-3 pb-2 text-small text-muted-foreground">
          {goal.status === "active"
            ? "No failed check since the watch began. That covers only runs the ledger witnessed."
            : "No alerts."}
          {dismissed > 0 && ` ${dismissed} dismissed.`}
        </p>
      ) : (
        <ul className="m-0 list-none p-0">
          {alerts.map((a) => (
            <li key={a.id}>
              <AlertRow goal={goal} alert={a} open={focus?.alertId === a.id} />
            </li>
          ))}
        </ul>
      )}

      <Thread goal={goal} />
    </section>
  );
}

function AlertRow({ goal, alert, open }: { goal: WatchGoal; alert: WatchAlert; open: boolean }) {
  const openAlert = useWatchStore((s) => s.openAlert);
  const closeAlert = useWatchStore((s) => s.closeAlert);
  const checks = alert.evidenceSeqs.length;
  return (
    <>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => (open ? closeAlert() : void openAlert(goal.id, alert.id))}
        data-testid="watch-alert"
        className={cn(
          "flex w-full flex-col gap-0.5 px-3 py-1.5 text-left transition-colors dur-1",
          "hover:bg-[var(--hover)] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
          open && "bg-[var(--accent-12)]",
        )}
      >
        <span className="flex items-center gap-2">
          <Chip tone="danger">failed</Chip>
          <span className="min-w-0 flex-1 truncate font-code text-small text-foreground">
            {alert.command || "(command not recorded)"}
          </span>
          {alert.state === "new" && <Chip tone="accent">new</Chip>}
        </span>
        <span className="text-caption text-muted-foreground">
          {time(alert.ts)}
          {alert.exitCode != null && ` · exit ${alert.exitCode}`}
          {` · revision ${alert.revision ? shortSha(alert.revision) : "unknown"}`}
          {checks > 1 && ` · ${checks} failed checks in this run`}
          {alert.recovered && " · happened while the app was closed"}
        </span>
      </button>
      {open && <Evidence goal={goal} alert={alert} />}
    </>
  );
}

/// One ledger line, in words. Only what the line itself states.
function describe(e: ProofEvent): string {
  const p = e.payload;
  switch (e.kind) {
    case "prompt":
      return str(p.prompt) || "(prompt not recorded)";
    case "snapshot":
      return `working tree at ${shortSha(str(p.commitSha))}`;
    case "tool_result":
      return [str(p.tool), str(p.command) || str(p.excerpt)].filter(Boolean).join(" · ");
    case "check_run": {
      const code = typeof p.exitCode === "number" ? ` (exit ${p.exitCode})` : "";
      return `${str(p.command)} → ${str(p.status)}${code}`;
    }
    case "turn_end": {
      const files = Array.isArray(p.filesChanged)
        ? (p.filesChanged as { path?: string }[]).map((f) => f.path).filter(Boolean)
        : [];
      return files.length ? `changed ${files.join(", ")}` : "run closed (no diff recorded)";
    }
    case "verification":
      return `${str(p.status)} — ${str(p.reason)}`;
    default:
      return "";
  }
}

function Evidence({ goal, alert }: { goal: WatchGoal; alert: WatchAlert }) {
  const evidence = useWatchStore((s) => s.evidence);
  const error = useWatchStore((s) => s.evidenceError);
  const dismissAlert = useWatchStore((s) => s.dismissAlert);
  const selectCase = useProofStore((s) => s.selectCase);
  const selectTurn = useShellStore((s) => s.selectTurn);

  const openInProof = () => {
    selectCase(alert.caseId);
    window.dispatchEvent(new CustomEvent("ac:open-workbench-tab", { detail: "proof" }));
    if (alert.turnId) selectTurn(alert.turnId);
  };

  return (
    <div className="border-l-2 border-primary bg-surface-2 px-3 py-2" data-testid="watch-evidence">
      <p className="m-0 text-caption font-semibold uppercase tracking-wide text-muted-foreground">
        Evidence · case {alert.caseId}
      </p>
      {error ? (
        <p className="m-0 py-1 text-small text-danger" role="alert">
          Could not read the ledger lines of this run: {error}
        </p>
      ) : evidence.length === 0 ? (
        <p className="m-0 py-1 text-small text-muted-foreground">Reading the ledger…</p>
      ) : (
        <ol className="m-0 list-none p-0 py-1">
          {evidence.map((e) => {
            const trigger = alert.evidenceSeqs.includes(e.seq);
            return (
              <li
                key={e.seq}
                className={cn(
                  "flex items-baseline gap-2 py-0.5 text-small",
                  trigger ? "text-foreground" : "text-muted-foreground",
                )}
              >
                <span className="w-16 shrink-0 whitespace-nowrap font-code text-caption tabular-nums">
                  {time(e.ts)}
                </span>
                <span className="w-14 shrink-0 font-code text-caption tabular-nums">
                  seq {e.seq}
                </span>
                <span className="w-24 shrink-0 font-code text-caption">{e.kind}</span>
                <span className={cn("min-w-0 flex-1 break-words", trigger && "font-semibold")}>
                  {describe(e)}
                  {trigger && <span className="sr-only"> (triggered this alert)</span>}
                </span>
              </li>
            );
          })}
        </ol>
      )}
      <div className="flex flex-wrap items-center gap-1.5 pt-1">
        {goal.source === "ledger" ? (
          <Button size="sm" onClick={openInProof} data-testid="watch-open-proof">
            Open this run in Proof
          </Button>
        ) : (
          <span className="text-caption text-muted-foreground">
            Demo events are scripted: they are not in your ledger, so there is nothing to open in
            Proof.
          </span>
        )}
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void dismissAlert(goal.id, alert.id).catch(fail)}
        >
          Dismiss
        </Button>
      </div>
    </div>
  );
}

function Thread({ goal }: { goal: WatchGoal }) {
  const ask = useWatchStore((s) => s.ask);
  const asking = useWatchStore((s) => s.asking);
  const openAlert = useWatchStore((s) => s.openAlert);
  const [question, setQuestion] = useState("");
  const recent = goal.thread.slice(-6);

  const send = async (text: string) => {
    if (!text.trim()) return;
    const ex = await ask(goal.id, text);
    if (ex) setQuestion("");
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    void send(question);
  };
  /// "Which one?" → open the chosen run and ask again about it.
  const choose = async (ex: WatchExchange, alertId: string) => {
    await openAlert(goal.id, alertId);
    await send(ex.question);
  };
  const label = (alertId: string) => {
    const a = goal.alerts.find((x) => x.id === alertId);
    return a ? `${time(a.ts)} · ${a.command}` : alertId;
  };

  return (
    <div className="px-3 pb-3 pt-1">
      <SectionHeader title="Ask" className="px-0" />
      {recent.length > 0 && (
        <ol className="m-0 flex list-none flex-col gap-2 p-0 pb-2" data-testid="watch-thread">
          {recent.map((ex, i) => (
            <li key={`${ex.ts}-${i}`} className="flex flex-col gap-0.5">
              <span className="text-small text-muted-foreground">You: {ex.question}</span>
              <span className="text-body text-foreground" data-testid="watch-answer">
                {ex.answer}
              </span>
              {ex.evidenceSeqs.length > 0 && (
                <span className="font-code text-caption text-muted-foreground">
                  from seq {ex.evidenceSeqs[0]}–{ex.evidenceSeqs[ex.evidenceSeqs.length - 1]}
                </span>
              )}
              {ex.candidates.length > 0 && (
                <span className="flex flex-wrap gap-1.5 pt-0.5">
                  {ex.candidates.map((id) => (
                    <Button key={id} size="sm" onClick={() => void choose(ex, id)}>
                      {label(id)}
                    </Button>
                  ))}
                </span>
              )}
            </li>
          ))}
        </ol>
      )}
      <form onSubmit={submit} className="flex items-center gap-2">
        <Input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          placeholder="¿Qué pasó en esa ejecución? · What happened in that run?"
          aria-label={`Ask about "${goal.title}"`}
          maxLength={500}
          data-testid="watch-question"
        />
        <Button type="submit" disabled={!question.trim()} loading={asking} data-testid="watch-ask">
          Ask
        </Button>
      </form>
    </div>
  );
}
