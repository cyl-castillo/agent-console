import { useEffect } from "react";

import { useProofStore, summarizeCases, buildTimeline } from "../stores/proofStore";
import { useSessionStore } from "../stores/sessionStore";
import type { TestigoPreviewEntry } from "../types/domain";

/// Public TSA used when the trusted-timestamp toggle is switched on. One
/// well-known default beats a URL field nobody fills correctly; the backend
/// takes any TSA URL if a project ever needs a different one.
const DEFAULT_TSA = "https://freetsa.org/tsr";

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function fmtTime(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts);
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/// Compact human summary of a preview entry's content, for the pre-sign
/// review: enough to decide "does this leak anything", without rendering the
/// full raw line.
function entryExcerpt(e: TestigoPreviewEntry): string {
  if (!e.line) return "";
  try {
    const v = JSON.parse(e.line) as { payload?: Record<string, unknown> };
    const p = v.payload ?? {};
    const s =
      typeof p.prompt === "string"
        ? p.prompt
        : typeof p.excerpt === "string"
          ? p.excerpt
          : JSON.stringify(p);
    return s.length > 180 ? `${s.slice(0, 180)}…` : s;
  } catch {
    return "";
  }
}

/// The Testigo surface: chain health + case list, per-case turn timelines,
/// and a pre-sign export review — every event is shown and can be manually
/// redacted BEFORE the packet is signed. Nothing leaves unreviewed.
export function ProofPanel() {
  const project = useSessionStore((s) => s.project);
  const events = useProofStore((s) => s.events);
  const report = useProofStore((s) => s.report);
  const exporting = useProofStore((s) => s.exporting);
  const lastExport = useProofStore((s) => s.lastExport);
  const error = useProofStore((s) => s.error);
  const selectedCase = useProofStore((s) => s.selectedCase);
  const review = useProofStore((s) => s.review);
  const settings = useProofStore((s) => s.settings);
  const setSettings = useProofStore((s) => s.setSettings);
  const load = useProofStore((s) => s.load);
  const startExport = useProofStore((s) => s.startExport);
  const toggleRedact = useProofStore((s) => s.toggleRedact);
  const confirmExport = useProofStore((s) => s.confirmExport);
  const cancelExport = useProofStore((s) => s.cancelExport);
  const selectCase = useProofStore((s) => s.selectCase);

  useEffect(() => {
    if (project) void load(project.root);
  }, [project, load]);

  const cases = summarizeCases(events);
  const timeline = selectedCase
    ? buildTimeline(events.filter((e) => e.caseId === selectedCase))
    : [];

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        {review ? (
          <span className="workbench-title">
            review before signing — {review.caseId ?? "full ledger"}
          </span>
        ) : selectedCase ? (
          <>
            <button
              className="workbench-action"
              onClick={() => selectCase(null)}
              title="Back to case list"
            >
              ←
            </button>
            <span className="workbench-title" title={selectedCase}>
              {selectedCase}
            </span>
          </>
        ) : (
          <span className="workbench-title">proof</span>
        )}
        {!review && report && (
          <span
            className={`wb-status ${report.ok ? "ok" : "off"}`}
            title={
              report.ok
                ? `Hash chain verified: ${report.total} events`
                : `Chain broken at seq ${report.brokenAtSeq}`
            }
          >
            {report.ok ? `chain ✓ ${report.total}` : "chain broken"}
          </span>
        )}
        <span className="spacer" />
        {!review && selectedCase && (
          <button
            className="workbench-action"
            disabled={exporting !== null}
            onClick={() => void startExport(selectedCase)}
            title="Review and export a signed proof packet for this case"
          >
            ⇩
          </button>
        )}
        {!review && (
          <button
            className="workbench-action"
            onClick={() => project && void load(project.root)}
            title="Re-verify the ledger"
          >
            ↻
          </button>
        )}
      </div>

      <div className="workbench-body">
        {error && <p className="wb-hint wb-error">{error}</p>}

        {review && (
          <section className="wb-section">
            <p className="wb-hint">
              This is everything the packet will contain. Token-shaped secrets
              are already auto-redacted; mark anything else that shouldn't
              leave this machine — redacted events keep their place in the
              verifiable chain, content excluded.
            </p>
            {review.preview.entries.map((e) => {
              const marked = review.redactSeqs.includes(e.seq);
              return (
                <div className="wb-row" key={e.seq}>
                  <span className="wb-row-main">
                    <code>
                      {e.seq} {e.kind}
                    </code>{" "}
                    {e.stub ? (
                      <span className="wb-hint">(other case — linkage stub only)</span>
                    ) : e.autoRedacted ? (
                      <span className="wb-hint">(auto-redacted) </span>
                    ) : null}
                    {!e.stub && (
                      <span className={`wb-hint${marked ? " proof-redacted" : ""}`}>
                        {marked ? "will be redacted" : entryExcerpt(e)}
                      </span>
                    )}
                  </span>
                  {!e.stub && (
                    <button
                      className="wb-link"
                      onClick={() => toggleRedact(e.seq)}
                      title={
                        marked
                          ? "Include this event's content"
                          : "Exclude this event's content from the packet"
                      }
                    >
                      {marked ? "keep" : "redact"}
                    </button>
                  )}
                </div>
              );
            })}
            <p>
              <button
                className="wb-cta"
                disabled={exporting !== null}
                onClick={() => void confirmExport()}
                title="Sign the packet with the chosen redactions"
              >
                {exporting !== null
                  ? "signing…"
                  : `sign & export${review.redactSeqs.length ? ` (${review.redactSeqs.length} redacted)` : ""}`}
              </button>{" "}
              <button className="wb-link" onClick={cancelExport}>
                cancel
              </button>
            </p>
          </section>
        )}

        {!review && events.length === 0 && !error && (
          <section className="wb-section">
            <p className="wb-hint">
              Prove your work, not just show it: every prompt, human approval
              and result lands here automatically, hash-chained. Export a
              signed <strong>proof packet</strong> and attach it to a PR, a
              client hand-off, an audit — anyone verifies it{" "}
              <a
                href="https://cyl-castillo.github.io/testigo/verifier/testigo-verifier.html"
                target="_blank"
                rel="noreferrer"
              >
                in a browser
              </a>
              , no install.
            </p>
            <p className="wb-hint">
              Nothing here yet — work with an agent and come back.
            </p>
          </section>
        )}

        {!review && !selectedCase && settings && (
          <section className="wb-section">
            <div className="wb-section-title">this project</div>
            <p className="wb-hint">
              <label>
                <input
                  type="checkbox"
                  checked={settings.witness}
                  onChange={(e) => void setSettings({ witness: e.target.checked })}
                />{" "}
                witness — record prompts, approvals and turn results in the
                local ledger (outside the repo, never pushed)
              </label>
            </p>
            <p className="wb-hint">
              <label>
                <input
                  type="checkbox"
                  checked={settings.repoMarks}
                  disabled={!settings.witness}
                  onChange={(e) => void setSettings({ repoMarks: e.target.checked })}
                />{" "}
                repo marks — stamp <code>Testigo-Case</code>/<code>Testigo-Head</code>{" "}
                commit trailers and pin the anchor ref in this project's git.
                Off by default: in shared repos this is the owner's call.
              </label>
            </p>
            <p className="wb-hint">
              <label>
                <input
                  type="checkbox"
                  checked={!!settings.timestampTsa}
                  onChange={(e) =>
                    void setSettings({
                      timestampTsa: e.target.checked ? DEFAULT_TSA : undefined,
                    })
                  }
                />{" "}
                trusted timestamp — at export, request an RFC 3161 token over
                the packet signature from <code>{hostOf(settings.timestampTsa ?? DEFAULT_TSA)}</code>{" "}
                (proves the packet existed at that time; sends the TSA only a
                signature hash, never content).
              </label>
            </p>
          </section>
        )}

        {!review && !selectedCase && cases.length > 0 && (
          <section className="wb-section">
            <div className="wb-section-title">cases</div>
            {cases.map((c) => (
              <div className="wb-row" key={c.caseId}>
                <button
                  className="wb-row-main wb-link"
                  onClick={() => selectCase(c.caseId)}
                  title={`Open the ${c.caseId} timeline`}
                >
                  <code>{c.caseId}</code>
                  <span className="wb-hint">
                    {" "}· {c.turns} turns · {c.approvals} approvals · {c.events} events
                  </span>
                </button>
                <button
                  className="wb-link"
                  disabled={exporting !== null}
                  onClick={() => void startExport(c.caseId)}
                  title="Review and export a signed proof packet for this case"
                >
                  export
                </button>
              </div>
            ))}
            <p className="wb-hint">
              <button
                className="wb-link"
                disabled={exporting !== null}
                onClick={() => void startExport(undefined)}
                title="Review and export the full ledger as one signed packet"
              >
                export full ledger
              </button>
            </p>
          </section>
        )}

        {!review && selectedCase && (
          <section className="wb-section">
            {timeline.length === 0 && (
              <p className="wb-hint">No turns recorded in this case yet.</p>
            )}
            {timeline.map((t) => (
              <div className="wb-section proof-turn" key={t.turnId ?? t.ts}>
                <p>
                  <span className="wb-hint">{fmtTime(t.ts)}</span>
                  {t.endTs === null && (
                    <span className="wb-hint"> · turn still open</span>
                  )}
                  <br />
                  <span title={t.prompt}>
                    {t.prompt.length > 160 ? `${t.prompt.slice(0, 160)}…` : t.prompt}
                  </span>
                </p>
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
                {(t.toolResults > 0 || t.files.length > 0) && (
                  <p className="wb-hint">
                    {t.toolResults > 0 && `${t.toolResults} tool calls`}
                    {t.toolResults > 0 && t.files.length > 0 && " · "}
                    {t.files.length > 0 &&
                      t.files
                        .slice(0, 8)
                        .map((f) => `${f.status} ${f.path}`)
                        .join(", ")}
                    {t.files.length > 8 && ` … +${t.files.length - 8} more`}
                    {t.filesTruncated && " (list capped at 500)"}
                  </p>
                )}
              </div>
            ))}
          </section>
        )}

        {!review && lastExport && (
          <section className="wb-section">
            <div className="wb-section-title">last export</div>
            <p className="wb-hint">
              <code>{lastExport.path}</code>
              <br />
              {lastExport.eventCount} events
              {lastExport.stubCount > 0 && ` · ${lastExport.stubCount} stubs`}
              {` · ${lastExport.redactionCount} redactions`}
              {lastExport.timestampTsa &&
                ` · timestamped (RFC 3161, ${hostOf(lastExport.timestampTsa)})`}
              {" · key "}
              <code title={lastExport.keyId}>{lastExport.keyId.slice(0, 16)}…</code>
              <br />
              Send the packet with <code>testigo-verifier.html</code> (written
              alongside) — the receiver verifies in a browser, no install.
            </p>
          </section>
        )}
      </div>
    </div>
  );
}
