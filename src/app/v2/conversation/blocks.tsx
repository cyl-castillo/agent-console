import { memo, useState } from "react";
import {
  ChevronRight,
  Copy,
  Info,
  MoreHorizontal,
  RotateCw,
  ShieldCheck,
  Scale,
} from "lucide-react";
import { Menu, type MenuItemDef } from "../../../ui/Menu";
import { cn } from "@/lib/cn";
import type { TimelineTurn, TimelineTool } from "../../../stores/proofStore";
import { Chip } from "../../../ui/Chip";
import { IconButton } from "../../../ui/IconButton";
import { MarkdownText } from "../../../ui/MarkdownText";
import { StatusGlyph } from "../../../ui/StatusGlyph";
import { Tooltip } from "../../../ui/Tooltip";
import { EMPTY_STREAM, saidText, type TurnStream } from "./stream";

/// "10:58 AM" today, "yesterday 10:58 AM", else "Oct 15, 10:58 AM".
const fmtTime = (ts: number, now: number) => {
  const d = new Date(ts);
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === new Date(now).toDateString()) return time;
  if (d.toDateString() === new Date(now - 86_400_000).toDateString()) return `yesterday ${time}`;
  return `${d.toLocaleDateString(undefined, { month: "short", day: "numeric" })} ${time}`;
};
/// Never negative (a clock behind the ledger's showed "-3s"), never "0s".
const fmtDuration = (ms: number) => {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s === 0) return "just now";
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
};

/// Tool calls, collapsed by default: the count and the last three are the
/// glance; opening shows every call with its command and exit.
export function ToolCallBlock({ tools, total }: { tools: TimelineTool[]; total: number }) {
  const [open, setOpen] = useState(false);
  if (total === 0) return null;
  const shown = open ? tools : tools.slice(-3);
  return (
    <div className="rounded-2 border border-border bg-surface-1" data-testid="tool-block">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="unbutton flex h-7 w-full items-center gap-1.5 px-2 text-left font-sans text-small text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
      >
        <ChevronRight
          size={14}
          className={cn("transition-transform dur-1", open && "rotate-90")}
          aria-hidden
        />
        <span>
          {total} tool call{total === 1 ? "" : "s"}
        </span>
        {!open && tools.length > 3 && <span className="text-faint-foreground">· last 3</span>}
      </button>
      <ul className="m-0 list-none border-t border-border p-0">
        {shown.map((t, i) => (
          <li key={i} className="flex items-baseline gap-2 px-2 py-1 font-code text-small">
            <span
              className={cn("w-12 shrink-0", t.failed ? "text-danger" : "text-muted-foreground")}
            >
              {t.tool}
            </span>
            <span className="min-w-0 flex-1 truncate text-foreground">
              {t.command ?? t.excerpt ?? ""}
            </span>
            {t.agentId && <Chip mono>subagent</Chip>}
            {t.exitCode !== undefined && t.exitCode !== 0 && (
              <Chip tone="danger" mono>{`exit ${t.exitCode}`}</Chip>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

/// Files the turn changed, each a jump into Changes at that file.
export function DiffBlock({
  files,
  truncated,
  onOpenFile,
}: {
  files: { status: string; path: string }[];
  truncated: boolean;
  onOpenFile: (path: string) => void;
}) {
  if (files.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5" data-testid="diff-block">
      {files.map((f) => (
        <button
          key={f.path}
          type="button"
          onClick={() => onOpenFile(f.path)}
          className="inline-flex h-6 items-center gap-1.5 rounded-1 border border-border bg-surface-1 px-1.5 font-code text-caption text-foreground hover:bg-[var(--hover)] focus-visible:outline-2 focus-visible:-outline-offset-1 focus-visible:outline-ring"
        >
          <span
            className={cn(
              "font-semibold",
              f.status === "A" ? "text-ok" : f.status === "D" ? "text-danger" : "text-warn",
            )}
          >
            {f.status}
          </span>
          {f.path}
        </button>
      ))}
      {truncated && <span className="text-caption text-faint-foreground">and more</span>}
    </div>
  );
}

/// What the ledger holds for the turn: checks, commits, the fact that it is
/// witnessed. Actions: copy the summary, re-run the prompt, open the ledger.
export function ProofBlock({
  turn,
  onCopy,
  onRerun,
  onLedger,
  onReview,
  menu,
}: {
  turn: TimelineTurn;
  onCopy: () => void;
  onRerun: () => void;
  onLedger: () => void;
  onReview?: () => void;
  /// F6: the same actions (and more) behind one tab stop.
  menu?: MenuItemDef[];
}) {
  return (
    <div
      className="flex flex-wrap items-center gap-2 border-t border-border pt-2 text-caption text-muted-foreground"
      data-testid="proof-block"
    >
      {turn.checks.map((c, i) => (
        <Chip
          key={i}
          tone={c.status === "passed" ? "ok" : c.status === "failed" ? "danger" : "warn"}
          mono
        >
          check {c.status === "passed" ? "✓" : c.status === "failed" ? "✕" : "∙"} {c.command}
        </Chip>
      ))}
      {turn.commits.map((c) => (
        <Chip key={c.sha} mono>
          commit {c.sha.slice(0, 7)} · {c.subject}
        </Chip>
      ))}
      {turn.injected.length > 0 && (
        <Tooltip content={turn.injected.join(", ")}>
          <span>
            <Chip tone="purple" mono>
              memory ×{turn.injected.length}
            </Chip>
          </span>
        </Tooltip>
      )}
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
      <span className="ml-auto inline-flex items-center gap-0.5">
        {onReview && (
          <Tooltip content="Ask the reviewer agent (a different identity) to review this turn">
            <IconButton
              size="sm"
              label="Review this turn"
              icon={<Scale size={14} />}
              onClick={onReview}
              tabIndex={-1}
            />
          </Tooltip>
        )}
        <Tooltip content="Copy the agent's summary">
          <IconButton
            size="sm"
            label="Copy summary"
            icon={<Copy size={14} />}
            onClick={onCopy}
            tabIndex={-1}
          />
        </Tooltip>
        <Tooltip content="Type this prompt again into the terminal">
          <IconButton
            size="sm"
            label="Re-run prompt"
            icon={<RotateCw size={14} />}
            onClick={onRerun}
            tabIndex={-1}
          />
        </Tooltip>
        <Tooltip content="Open this turn in the ledger">
          <IconButton
            size="sm"
            label="Open in ledger"
            icon={<ShieldCheck size={14} />}
            onClick={onLedger}
            tabIndex={-1}
          />
        </Tooltip>
        {menu && (
          <Menu
            items={menu}
            trigger={
              <IconButton
                size="sm"
                label="More actions for this turn"
                icon={<MoreHorizontal size={14} />}
                data-testid="turn-menu"
              />
            }
          />
        )}
      </span>
    </div>
  );
}

/// Who is speaking: "You" over the prompt, "Agent" over the reply.
function SpeakerLabel({ children }: { children: string }) {
  return (
    <span className="font-sans text-caption font-medium uppercase tracking-wide text-faint-foreground">
      {children}
    </span>
  );
}

/// One turn: the prompt, what the agent did, what it changed, what it said,
/// and the proof line. A turn without an end is live. Memoised (F6): the
/// handlers take the turn, so the parent passes the same functions to every
/// row and only the live turn re-renders with the clock.
export const TurnBlock = memo(function TurnBlock({
  turn,
  live,
  now,
  onOpenFile,
  onCopy,
  onRerun,
  onLedger,
  onReview,
  onMore,
  waitingApproval,
  stream = EMPTY_STREAM,
  onCopyText,
  selected,
  onSelect,
}: {
  turn: TimelineTurn;
  live: boolean;
  now: number;
  onOpenFile: (path: string) => void;
  onCopy: (turn: TimelineTurn) => void;
  onRerun: (turn: TimelineTurn) => void;
  onLedger: (turn: TimelineTurn) => void;
  onReview?: (turn: TimelineTurn) => void;
  /// Where the rest of a cut summary lives (the terminal).
  onMore?: () => void;
  /// F6: this turn is the one the Inspector details.
  selected?: boolean;
  onSelect?: (turn: TimelineTurn) => void;
  /// F3: an approval of this session is pending — the agent is not working,
  /// it is waiting for the human.
  waitingApproval?: boolean;
  /// F5: what the transcript says about this turn — the agent's paragraphs
  /// as they are written and the tool call still running.
  stream?: TurnStream;
  /// Copy a code fence from the agent's prose.
  onCopyText?: (text: string) => void;
}) {
  const said = saidText(stream, turn.summary);
  const menu: MenuItemDef[] = [
    ...(onSelect ? [{ label: "Details", icon: <Info />, onSelect: () => onSelect(turn) }] : []),
    { label: "Copy summary", icon: <Copy />, onSelect: () => onCopy(turn) },
    { label: "Re-run prompt", icon: <RotateCw />, onSelect: () => onRerun(turn) },
    ...(onReview
      ? [{ label: "Review this turn", icon: <Scale />, onSelect: () => onReview(turn) }]
      : []),
    { label: "Open in ledger", icon: <ShieldCheck />, onSelect: () => onLedger(turn) },
  ];
  const state = turn.failed ? "failed" : turn.interrupted ? "stopped" : live ? "working" : "done";
  const duration = turn.endTs ? turn.endTs - turn.ts : now - turn.ts;
  return (
    <article
      className={cn(
        "flex flex-col gap-2 border-l-2 px-4 py-3",
        selected ? "border-primary" : "border-transparent",
      )}
      data-testid="turn"
      data-live={live || undefined}
      data-selected={selected || undefined}
      onClick={onSelect ? () => onSelect(turn) : undefined}
    >
      <header className="flex items-center gap-2 font-sans text-caption text-faint-foreground">
        <StatusGlyph state={state} />
        <span className="font-code tabular-nums">{fmtTime(turn.ts, now)}</span>
        <span>·</span>
        <span className="font-code tabular-nums">{fmtDuration(duration)}</span>
        {turn.model && <Chip mono>{turn.model}</Chip>}
        {turn.skill && <Chip tone="accent" mono>{`/${turn.skill}`}</Chip>}
        {turn.interrupted && (
          <Tooltip
            content={
              turn.superseded
                ? "Never closed on its own — the next prompt ended it"
                : "Cut short by Interrupt"
            }
          >
            <span>
              <Chip tone="warn">interrupted</Chip>
            </span>
          </Tooltip>
        )}
        {turn.rewound && <Chip tone="warn">rewound here</Chip>}
        {turn.approvals.length > 0 && (
          <span className="ml-auto">
            {turn.approvals.length} approval{turn.approvals.length === 1 ? "" : "s"}
          </span>
        )}
      </header>
      <div className="flex flex-col gap-1" data-testid="you-block">
        <SpeakerLabel>You</SpeakerLabel>
        <div className="whitespace-pre-wrap break-words rounded-2 bg-surface-2 px-3 py-2 font-sans text-body text-foreground">
          {turn.prompt || "(no prompt recorded)"}
        </div>
      </div>
      <ToolCallBlock tools={turn.tools} total={turn.toolResults} />
      <DiffBlock files={turn.files} truncated={turn.filesTruncated} onOpenFile={onOpenFile} />
      {said && (
        <div className="flex flex-col gap-1" data-testid="agent-block">
          <SpeakerLabel>Agent</SpeakerLabel>
          <MarkdownText text={said} onOpenFile={onOpenFile} onCopy={onCopyText} />
        </div>
      )}
      {turn.summaryTruncated && stream.texts.length === 0 && (
        <button
          type="button"
          onClick={onMore}
          className="unbutton self-start font-sans text-small text-muted-foreground underline decoration-dotted underline-offset-2 hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring"
          data-testid="summary-more"
        >
          … the agent said more — the full reply is in the terminal (Ctrl+`)
        </button>
      )}
      {turn.failed && (
        <div
          className="rounded-2 border border-danger/40 bg-[var(--danger-08)] px-2 py-1 font-sans text-small text-foreground"
          role="alert"
        >
          The turn did not finish: {turn.error ?? "the API refused it"}
          {turn.errorDetails && (
            <span className="text-muted-foreground"> — {turn.errorDetails}</span>
          )}
        </div>
      )}
      {live && (
        <div className="font-sans text-small text-muted-foreground" data-testid="live-status">
          {waitingApproval ? (
            <span className="text-warn">Waiting for your approval below</span>
          ) : stream.running ? (
            <span className="inline-flex max-w-full items-baseline gap-1.5">
              <span className="shrink-0">Running {stream.running.name}</span>
              {stream.running.input && (
                <span className="min-w-0 truncate font-code text-caption text-faint-foreground">
                  {stream.running.input}
                </span>
              )}
            </span>
          ) : (
            <>
              Working… {Math.max(turn.toolResults, stream.toolUses)} tool call
              {Math.max(turn.toolResults, stream.toolUses) === 1 ? "" : "s"} so far
            </>
          )}
        </div>
      )}
      {!live && (
        <ProofBlock
          turn={turn}
          onCopy={() => onCopy(turn)}
          onRerun={() => onRerun(turn)}
          onLedger={() => onLedger(turn)}
          onReview={onReview ? () => onReview(turn) : undefined}
          menu={menu}
        />
      )}
    </article>
  );
});
