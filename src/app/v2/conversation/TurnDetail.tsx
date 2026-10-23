import { Copy, RotateCw, ShieldCheck, Undo2 } from "lucide-react";
import { profileFor } from "../../../agents/profiles";
import { cn } from "@/lib/cn";
import { copyText } from "../../../lib/clipboard";
import { typeIntoActiveSession } from "../../../lib/termInput";
import { useChangesStore } from "../../../stores/changesStore";
import { confirmDialog } from "../../../stores/confirmStore";
import { freshStatus, useLiveStatusStore } from "../../../stores/liveStatusStore";
import { useProofStore, type TimelineTurn } from "../../../stores/proofStore";
import { checkGlyph, checkTitle, checkTone } from "./checkStatus";
import { useShellStore } from "../../../stores/shellStore";
import { useToastStore } from "../../../stores/toastStore";
import { itemsFor, useTranscriptStore } from "../../../stores/transcriptStore";
import { useUIStore } from "../../../stores/uiStore";
import { Button } from "../../../ui/Button";
import { Chip } from "../../../ui/Chip";
import { EmptyState } from "../../../ui/EmptyState";
import { MarkdownText } from "../../../ui/MarkdownText";
import { Tooltip } from "../../../ui/Tooltip";
import { saidText, streamFor } from "./stream";
import { useSessionTimeline } from "./useSessionTimeline";

const fmtTime = (ts: number) =>
  new Date(ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
const fmtDuration = (ms: number) => {
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
};
const fmtTokens = (n: number) =>
  n < 1000 ? String(n) : `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;

function Section({
  title,
  children,
  testId,
}: {
  title: string;
  children: React.ReactNode;
  testId?: string;
}) {
  return (
    <section className="flex flex-col gap-1" data-testid={testId}>
      <h3 className="m-0 font-sans text-caption font-semibold uppercase tracking-wide text-faint-foreground">
        {title}
      </h3>
      {children}
    </section>
  );
}

/// F6: the Inspector under Sessions is the selected turn in full — every
/// tool call with its command and exit, the approvals, the checks, the
/// files, what the agent said, the commits, and the actions that act on the
/// turn (copy, re-run, rewind, ledger). The thread stays the thread; this is
/// its detail, so nothing is listed twice.
export function TurnDetail() {
  const { session, turns } = useSessionTimeline();
  const selectedId = useShellStore((s) => s.selectedTurnId);
  const turn: TimelineTurn | null =
    (selectedId && turns.find((t) => t.turnId === selectedId)) || turns[turns.length - 1] || null;
  const live = useLiveStatusStore((s) =>
    session ? freshStatus(s.byTerm, session.id, Date.now()) : null,
  );
  const transcript = useTranscriptStore(itemsFor(session?.agentSessionId));
  const rewindToTurn = useProofStore((s) => s.rewindToTurn);
  const selectCase = useProofStore((s) => s.selectCase);
  const setSelected = useChangesStore((s) => s.setSelected);
  const setTab = useUIStore((s) => s.setTab);
  const showToast = useToastStore((s) => s.show);
  if (!session || !turn) {
    return (
      <EmptyState
        icon={<ShieldCheck />}
        title="No turn yet"
        description="Each turn of the conversation lands here in full: tools, approvals, files, what the agent said."
      />
    );
  }
  const stream = streamFor(transcript, turn);
  const said = saidText(stream, turn.summary);
  const isLive = turn.endTs === null;
  const rewindable =
    profileFor(session.agent).supportsTranscriptFork &&
    !!turn.postSha &&
    !!turn.sessionId &&
    !isLive;
  const srcLive = session.status === "live";
  const openFile = (path: string) => {
    void setSelected(path);
    setTab("changes");
  };
  const rewind = async () => {
    const ok = await confirmDialog({
      title: "Rewind to this turn?",
      message:
        "Files are restored to how they were when this turn ended — changes from later turns are discarded. A backup snapshot is taken first (undo via ⌘P → Undo last restore).\n\nA new session opens, resuming the conversation as of this turn. The current session and its transcript are kept untouched as history.",
      confirmLabel: "Rewind",
      danger: true,
    });
    if (ok) await rewindToTurn(turn);
  };
  const openLedger = () => {
    if (turn.caseId) selectCase(turn.caseId);
    window.dispatchEvent(new CustomEvent("ac:open-workbench-tab", { detail: "proof" }));
  };
  const copy = (text: string, what: string) =>
    void copyText(text).then(
      () => showToast(`${what} copied`, "success"),
      (e) => showToast(`Copy failed: ${String(e)}`, "error"),
    );
  const ctx = live && (live.contextUsed ?? 0) > 0 ? fmtTokens(live.contextUsed ?? 0) : null;
  return (
    <div className="flex flex-col gap-3 p-3 font-sans text-body" data-testid="turn-detail">
      <div className="flex flex-wrap items-center gap-1.5 text-caption text-muted-foreground">
        <span className="font-code tabular-nums">{fmtTime(turn.ts)}</span>
        <span>·</span>
        <span className="font-code tabular-nums">
          {isLive ? "working…" : fmtDuration((turn.endTs ?? turn.ts) - turn.ts)}
        </span>
        {turn.model && <Chip mono>{turn.model}</Chip>}
        {turn.failed && <Chip tone="danger">failed{turn.error ? ` · ${turn.error}` : ""}</Chip>}
        {turn.interrupted && <Chip tone="warn">interrupted</Chip>}
        {turn.rewound && <Chip tone="warn">rewound here</Chip>}
        {turn.skill && <Chip tone="accent" mono>{`/${turn.skill}`}</Chip>}
        {live?.costUsd !== undefined && (
          <Tooltip content="Session cost so far, from the CLI's status line">
            <span>
              <Chip mono>${live.costUsd.toFixed(2)}</Chip>
            </span>
          </Tooltip>
        )}
        {ctx && (
          <Tooltip content="Context in use, from the CLI's status line">
            <span>
              <Chip mono>⌁ {ctx}</Chip>
            </span>
          </Tooltip>
        )}
      </div>
      <Section title="You" testId="detail-prompt">
        <div className="whitespace-pre-wrap break-words rounded-2 bg-surface-2 px-2 py-1.5 text-foreground">
          {turn.prompt || "(no prompt recorded)"}
        </div>
        {turn.injected.length > 0 && (
          <div className="text-caption text-muted-foreground">
            ◈ {turn.injected.map((d) => d.replace(/^(memory|skill):/, "")).join(", ")}
          </div>
        )}
      </Section>
      {turn.approvals.length > 0 && (
        <Section title={`Approvals · ${turn.approvals.length}`}>
          <ul className="m-0 flex list-none flex-col gap-0.5 p-0 text-small">
            {turn.approvals.map((a, i) => (
              <li key={i}>
                <span className="font-code">{a.tool ?? "?"}</span> {a.decision}
                {a.reason && <span className="text-muted-foreground"> — {a.reason}</span>}
              </li>
            ))}
          </ul>
        </Section>
      )}
      {turn.toolResults > 0 && (
        <Section title={`Tool calls · ${turn.toolResults}`} testId="detail-tools">
          <ul className="m-0 flex list-none flex-col gap-0.5 p-0 font-code text-small">
            {turn.tools.map((t, i) => (
              <li key={i} className="flex flex-col gap-0.5">
                <div className="flex items-baseline gap-2">
                  <span
                    className={cn("shrink-0", t.failed ? "text-danger" : "text-muted-foreground")}
                  >
                    {t.failed ? "✕ " : ""}
                    {t.tool}
                    {t.agentId ? " (subagent)" : ""}
                  </span>
                  <span className="min-w-0 flex-1 break-all text-foreground">
                    {t.command ?? ""}
                  </span>
                  {t.failed && t.exitCode !== undefined && (
                    <span className="shrink-0 text-danger">exit {t.exitCode}</span>
                  )}
                </div>
                {t.excerpt && (
                  <pre className="m-0 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-1 bg-surface-2 px-1.5 py-1 text-caption text-muted-foreground">
                    {t.excerpt}
                  </pre>
                )}
              </li>
            ))}
            {turn.tools.length < turn.toolResults && (
              <li className="text-muted-foreground">
                … {turn.toolResults - turn.tools.length} more not kept by the ledger
              </li>
            )}
          </ul>
        </Section>
      )}
      {turn.checks.length > 0 && (
        <Section title="Checks">
          <div className="flex flex-wrap gap-1">
            {turn.checks.map((c, i) => (
              <Chip key={i} mono tone={checkTone(c.status)} title={checkTitle(c.status)}>
                {c.kind ?? "check"} {checkGlyph(c.status)} {c.command}
                {c.status === "failed" && c.exitCode !== undefined ? ` exit ${c.exitCode}` : ""}
                {c.status === "empty" ? " · ran 0 tests" : ""}
                {c.status === "unknown" ? " · exit unknown" : ""}
              </Chip>
            ))}
          </div>
        </Section>
      )}
      {turn.files.length > 0 && (
        <Section
          title={`Files · ${turn.files.length}${turn.filesTruncated ? "+" : ""}`}
          testId="detail-files"
        >
          <ul className="m-0 flex list-none flex-col p-0 font-code text-small">
            {turn.files.map((f) => (
              <li key={f.path}>
                <button
                  type="button"
                  onClick={() => openFile(f.path)}
                  className="unbutton flex w-full items-baseline gap-2 rounded-1 px-1 py-0.5 text-left hover:bg-[var(--hover)] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
                >
                  <span
                    className={cn(
                      "w-3 shrink-0",
                      f.status === "A" ? "text-ok" : f.status === "D" ? "text-danger" : "text-warn",
                    )}
                  >
                    {f.status}
                  </span>
                  <span className="min-w-0 flex-1 truncate">{f.path}</span>
                </button>
              </li>
            ))}
          </ul>
        </Section>
      )}
      {said && (
        <Section title="Agent" testId="detail-said">
          <MarkdownText text={said} onOpenFile={openFile} onCopy={(t) => copy(t, "Code")} />
        </Section>
      )}
      {turn.commits.length > 0 && (
        <Section title="Commits">
          <ul className="m-0 flex list-none flex-col gap-0.5 p-0 font-code text-small">
            {turn.commits.map((c) => (
              <li key={c.sha}>
                commit {c.sha.slice(0, 7)} · {c.subject}
                {c.amend ? " (amend)" : ""}
              </li>
            ))}
          </ul>
        </Section>
      )}
      {(turn.review || turn.budget) && (
        <Section title="Decisions">
          <div className="flex flex-wrap gap-1">
            {turn.review && (
              <Chip
                tone={
                  turn.review.verdict === "block"
                    ? "danger"
                    : turn.review.verdict === "approve"
                      ? "ok"
                      : "warn"
                }
                mono
              >
                review {turn.review.verdict}
                {turn.review.decision ? ` → ${turn.review.decision}` : ""}
              </Chip>
            )}
            {turn.budget && (
              <Chip tone="warn" mono>
                budget ${turn.budget.spentUsd.toFixed(2)} / ${turn.budget.capUsd.toFixed(2)}
                {turn.budget.decision ? ` → ${turn.budget.decision}` : ""}
              </Chip>
            )}
          </div>
        </Section>
      )}
      <div
        className="flex flex-wrap gap-1 border-t border-border pt-2"
        data-testid="detail-actions"
      >
        <Button
          size="sm"
          variant="ghost"
          icon={<Copy size={14} />}
          onClick={() => copy(turn.prompt, "Prompt")}
        >
          Copy prompt
        </Button>
        <Button
          size="sm"
          variant="ghost"
          icon={<RotateCw size={14} />}
          onClick={() => void typeIntoActiveSession(turn.prompt)}
        >
          Re-run
        </Button>
        {rewindable && (
          <Tooltip
            content={
              srcLive
                ? "Stop the session first — a live agent would keep writing over the restored files"
                : "Restore the files AND the conversation to the end of this turn"
            }
          >
            <span>
              <Button
                size="sm"
                variant="ghost"
                icon={<Undo2 size={14} />}
                disabled={srcLive}
                onClick={() => void rewind()}
              >
                Rewind
              </Button>
            </span>
          </Tooltip>
        )}
        <Button size="sm" variant="ghost" icon={<ShieldCheck size={14} />} onClick={openLedger}>
          Ledger
        </Button>
      </div>
    </div>
  );
}
