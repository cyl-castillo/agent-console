import { useEffect, useMemo, useState } from "react";

import { profileFor } from "../agents/profiles";
import { typeIntoActiveSession } from "../lib/termInput";
import { useChangesStore } from "../stores/changesStore";
import { confirmDialog } from "../stores/confirmStore";
import { freshStatus, useLiveStatusStore } from "../stores/liveStatusStore";
import { buildTimeline, useProofStore, type TimelineTurn } from "../stores/proofStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";
import { useUIStore } from "../stores/uiStore";

/// The active session, turn by turn — what the terminal shows as raw
/// scrollback, as structure: prompt, what the console fed it, the tools it
/// ran (and which failed), the tests it ran, the files it changed, what it
/// said when it stopped, what you committed from it. Everything here is read
/// from the Testigo ledger (the same evidence a proof packet carries) plus the
/// CLI's own status line for cost/context; nothing is scraped from the PTY.
/// The Ledger sub-tab is the audit view of the same data across all sessions.

function fmtTime(ts: number): string {
  if (!ts) return "";
  return new Date(ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

function fmtDuration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${String(s % 60).padStart(2, "0")}s`;
}

function fmtTokens(n: number): string {
  if (n < 1000) return String(n);
  const k = n / 1000;
  return `${k >= 10 ? Math.round(k) : k.toFixed(1)}k`;
}

const PROMPT_FOLD = 320;
const TOOLS_FOLD = 8;

export function TurnsPanel() {
  const project = useSessionStore((s) => s.project);
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const events = useProofStore((s) => s.events);
  const proofRoot = useProofStore((s) => s.projectRoot);
  const load = useProofStore((s) => s.load);
  const rewindToTurn = useProofStore((s) => s.rewindToTurn);
  const selectCase = useProofStore((s) => s.selectCase);
  const setSelected = useChangesStore((s) => s.setSelected);
  const setTab = useUIStore((s) => s.setTab);
  const active = sessions.find((s) => s.id === activeId) ?? null;
  const live = useLiveStatusStore((s) =>
    active ? freshStatus(s.byTerm, active.id, Date.now()) : null,
  );
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  // A 1 s tick keeps "working… 42s" and the open turn's elapsed time moving.
  const [, tick] = useState(0);

  useEffect(() => {
    if (project && proofRoot !== project.root) void load(project.root);
  }, [project, proofRoot, load]);

  const turns = useMemo(() => {
    if (!active) return [];
    // Events bound to this terminal, plus turn-bound events that carry no
    // terminal of their own (a `commit` is the human's act, recorded against
    // the turn whose diff it shipped).
    const turnIds = new Set(
      events.filter((e) => e.termId === active.id && e.turnId).map((e) => e.turnId as string),
    );
    const mine = events.filter(
      (e) => e.termId === active.id || (!e.termId && !!e.turnId && turnIds.has(e.turnId)),
    );
    // Newest first: what just happened is what you came to look at.
    return buildTimeline(mine).reverse();
  }, [events, active]);

  const open = turns.find((t) => t.endTs === null) ?? null;
  useEffect(() => {
    if (!open) return;
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [open]);

  if (!project) return null;

  const copyPrompt = async (t: TimelineTurn) => {
    try {
      const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
      await writeText(t.prompt);
      useToastStore.getState().show("Prompt copied", "success");
    } catch (e) {
      useToastStore.getState().show(`Copy failed: ${e}`, "error");
    }
  };

  const openFile = async (path: string) => {
    await setSelected(path);
    setTab("changes");
  };

  const rewind = async (t: TimelineTurn) => {
    const ok = await confirmDialog({
      title: "Rewind to this turn?",
      message:
        "Files are restored to how they were when this turn ended — changes from later turns are discarded. A backup snapshot is taken first (undo via ⌘P → Undo last restore).\n\nA new session opens, resuming the conversation as of this turn. The current session and its transcript are kept untouched as history.",
      confirmLabel: "Rewind",
      danger: true,
    });
    if (ok) await rewindToTurn(t);
  };

  const openLedger = (t: TimelineTurn) => {
    if (t.caseId) selectCase(t.caseId);
    window.dispatchEvent(new CustomEvent("ac:open-workbench-tab", { detail: "proof" }));
  };

  const model = live?.modelName ?? turns.find((t) => t.model)?.model;
  const cost = live?.costUsd !== undefined ? `$${live.costUsd.toFixed(2)}` : null;
  const ctx =
    live && (live.contextUsed ?? 0) > 0
      ? `${fmtTokens(live.contextUsed ?? 0)}${live.usedPct !== undefined ? ` (${Math.round(live.usedPct)}%)` : ""}`
      : null;

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title" title={active ? active.name : "no session"}>
          {active ? active.name : "turns"}
        </span>
        <span className="spacer" />
        {model && (
          <span className="wb-hint turns-meta" title="Model the CLI reports for this session">
            {model}
          </span>
        )}
        {ctx && (
          <span className="wb-hint turns-meta" title="Context in use (from the CLI's status line)">
            ⌁ {ctx}
          </span>
        )}
        {cost && (
          <span className="wb-hint turns-meta" title="Estimated session cost at list price">
            {cost}
          </span>
        )}
        <button
          className="workbench-action"
          onClick={() => void load(project.root)}
          title="Reload from the ledger"
        >
          ↻
        </button>
      </div>

      <div className="workbench-body">
        {!active && <p className="wb-hint">Select a session on the left to see its turns.</p>}
        {active && turns.length === 0 && (
          <p className="wb-hint">
            No turns recorded for this session yet. Send a prompt in the terminal — each turn lands
            here as it happens (prompt, tools, tests, files, approvals). If prompts go by and
            nothing shows up, check the <code>hooks</code> pill in the status bar.
          </p>
        )}
        {turns.map((t) => {
          const key = t.turnId ?? String(t.ts);
          const isOpen = t.endTs === null;
          const src = t.termId ? sessions.find((s) => s.id === t.termId) : undefined;
          const rewindable =
            !!src &&
            profileFor(src.agent).supportsTranscriptFork &&
            !!t.postSha &&
            !!t.sessionId &&
            t.endTs !== null;
          const srcLive = src?.status === "live";
          const promptLong = t.prompt.length > PROMPT_FOLD;
          const showAll = !!expanded[key];
          const tools = showAll ? t.tools : t.tools.slice(0, TOOLS_FOLD);
          const failedTools = t.tools.filter((x) => x.failed).length;
          return (
            <section
              className={`wb-section turns-card${isOpen ? " turns-open" : ""}${t.failed ? " turns-failed" : ""}`}
              key={key}
            >
              <p className="turns-head">
                <span className="wb-hint">{fmtTime(t.ts)}</span>
                {isOpen ? (
                  <span className="turns-status turns-status-open">
                    {" "}
                    · working… {fmtDuration(Date.now() - t.ts)}
                  </span>
                ) : (
                  t.endTs !== null && (
                    <span className="wb-hint"> · {fmtDuration(t.endTs - t.ts)}</span>
                  )
                )}
                {t.failed && (
                  <span className="wb-error" title={t.errorDetails}>
                    {" "}
                    · failed{t.error ? ` — ${t.error}` : ""}
                  </span>
                )}
                {t.rewound && (
                  <span className="wb-hint" title="History was rewound to the end of this turn">
                    {" "}
                    · ↶ rewound here
                  </span>
                )}
                {t.skill && (
                  <span className="wb-hint">
                    {" "}
                    · <code>/{t.skill}</code>
                  </span>
                )}
              </p>

              <pre className="turns-prompt" title={promptLong && !showAll ? t.prompt : undefined}>
                {promptLong && !showAll ? `${t.prompt.slice(0, PROMPT_FOLD)}…` : t.prompt}
              </pre>

              {t.injected.length > 0 && (
                <p
                  className="wb-hint turns-injected"
                  title="Memories/skills the console fed this prompt"
                >
                  ◈ {t.injected.map((d) => d.replace(/^(memory|skill):/, "")).join(", ")}
                </p>
              )}

              {t.approvals.length > 0 && (
                <p className="wb-hint">
                  {t.approvals.map((a, i) => (
                    <span key={i}>
                      {i > 0 && " · "}
                      <code>{a.tool ?? "?"}</code> {a.decision}
                      {a.reason ? ` — ${a.reason}` : ""}
                    </span>
                  ))}
                </p>
              )}

              {t.tools.length > 0 && (
                <ul className="turns-tools">
                  {tools.map((x, i) => (
                    <li
                      key={i}
                      className={x.failed ? "turns-tool-failed" : undefined}
                      title={x.excerpt ? x.excerpt.slice(0, 400) : undefined}
                    >
                      <span className="turns-tool-name">
                        {x.failed ? "✗ " : ""}
                        {x.tool}
                        {x.agentId ? " (subagent)" : ""}
                      </span>
                      {x.command ? (
                        <code className="turns-tool-cmd">
                          {x.command.length > 90 ? `${x.command.slice(0, 90)}…` : x.command}
                        </code>
                      ) : (
                        x.excerpt && (
                          <span className="turns-tool-excerpt">
                            {x.excerpt.split("\n")[0].slice(0, 90)}
                          </span>
                        )
                      )}
                      {x.failed && x.exitCode !== undefined && (
                        <span className="wb-error"> exit {x.exitCode}</span>
                      )}
                    </li>
                  ))}
                  {t.tools.length > TOOLS_FOLD && !showAll && (
                    <li>
                      <button
                        className="wb-link"
                        onClick={() => setExpanded((m) => ({ ...m, [key]: true }))}
                      >
                        … {t.toolResults - TOOLS_FOLD} more tool call
                        {t.toolResults - TOOLS_FOLD === 1 ? "" : "s"}
                        {failedTools > 0 ? ` (${failedTools} failed)` : ""}
                      </button>
                    </li>
                  )}
                </ul>
              )}

              {t.checks.length > 0 && (
                <p className="wb-hint">
                  {t.checks.map((c, i) => (
                    <span
                      key={i}
                      className={`proof-check proof-check-${c.status}`}
                      title={c.command}
                    >
                      {i > 0 && " · "}
                      {c.status === "passed" ? "✓" : c.status === "failed" ? "✗" : "⏸"}{" "}
                      <code>
                        {c.command.length > 48 ? `${c.command.slice(0, 48)}…` : c.command}
                      </code>
                      {c.status === "failed" && c.exitCode !== undefined && ` exit ${c.exitCode}`}
                    </span>
                  ))}
                </p>
              )}

              {t.files.length > 0 && (
                <p className="wb-hint turns-files">
                  {t.files.slice(0, showAll ? t.files.length : 8).map((f, i) => (
                    <span key={f.path}>
                      {i > 0 && " · "}
                      <button
                        className="wb-link"
                        title={`Open ${f.path} in Changes`}
                        onClick={() => void openFile(f.path)}
                      >
                        {f.status} {f.path}
                      </button>
                    </span>
                  ))}
                  {t.files.length > 8 && !showAll && (
                    <>
                      {" "}
                      <button
                        className="wb-link"
                        onClick={() => setExpanded((m) => ({ ...m, [key]: true }))}
                      >
                        … +{t.files.length - 8} more
                      </button>
                    </>
                  )}
                  {t.filesTruncated && " (list capped at 500)"}
                </p>
              )}

              {t.summary && (
                <p className="wb-hint proof-turn-summary" title={t.summary}>
                  ↳ {showAll || t.summary.length <= 220 ? t.summary : `${t.summary.slice(0, 220)}…`}
                  {t.summaryTruncated && " …"}
                </p>
              )}

              {t.commits.length > 0 && (
                <p className="wb-hint proof-turn-commits">
                  {t.commits.map((c, i) => (
                    <span key={i} title={`${c.files} file(s)${c.amend ? " · amend" : ""}`}>
                      {i > 0 && " · "}⎇ committed <code>{c.sha.slice(0, 7)}</code>
                      {c.subject ? ` — ${c.subject}` : ""}
                      {c.amend ? " (amend)" : ""}
                    </span>
                  ))}
                </p>
              )}

              <p className="turns-actions">
                {(promptLong || t.tools.length > TOOLS_FOLD || t.files.length > 8) && (
                  <button
                    className="wb-link"
                    onClick={() => setExpanded((m) => ({ ...m, [key]: !showAll }))}
                  >
                    {showAll ? "collapse" : "expand"}
                  </button>
                )}
                <button
                  className="wb-link"
                  onClick={() => void copyPrompt(t)}
                  title="Copy the prompt"
                >
                  copy
                </button>
                <button
                  className="wb-link"
                  onClick={() => void typeIntoActiveSession(t.prompt)}
                  title="Type this prompt into the active session again (review, then send)"
                >
                  re-run
                </button>
                {rewindable && (
                  <button
                    className="wb-link"
                    disabled={srcLive}
                    title={
                      srcLive
                        ? "Stop the session first — a live agent would keep writing over the restored files"
                        : "Restore the files AND the conversation to the end of this turn"
                    }
                    onClick={() => void rewind(t)}
                  >
                    ↶ rewind
                  </button>
                )}
                <button
                  className="wb-link"
                  onClick={() => openLedger(t)}
                  title="Open this turn's case in the ledger"
                >
                  ledger
                </button>
              </p>
            </section>
          );
        })}
      </div>
    </div>
  );
}
