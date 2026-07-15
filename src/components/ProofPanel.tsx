import { useEffect } from "react";

import { useProofStore, summarizeCases, buildTimeline } from "../stores/proofStore";
import { useSessionStore } from "../stores/sessionStore";

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

/// The Testigo surface: chain health + case list, and per-case a timeline of
/// turns (intent → approvals → results → files changed). Export from either
/// level produces the signed packet + standalone verifier.
export function ProofPanel() {
  const project = useSessionStore((s) => s.project);
  const events = useProofStore((s) => s.events);
  const report = useProofStore((s) => s.report);
  const exporting = useProofStore((s) => s.exporting);
  const lastExport = useProofStore((s) => s.lastExport);
  const error = useProofStore((s) => s.error);
  const selectedCase = useProofStore((s) => s.selectedCase);
  const load = useProofStore((s) => s.load);
  const exportPack = useProofStore((s) => s.exportPack);
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
        {selectedCase ? (
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
        {report && (
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
        {selectedCase && (
          <button
            className="workbench-action"
            disabled={exporting !== null}
            onClick={() => void exportPack(selectedCase)}
            title="Export a signed proof packet for this case"
          >
            {exporting === selectedCase ? "…" : "⇩"}
          </button>
        )}
        <button
          className="workbench-action"
          onClick={() => project && void load(project.root)}
          title="Re-verify the ledger"
        >
          ↻
        </button>
      </div>

      <div className="workbench-body">
        {error && <p className="wb-hint wb-error">{error}</p>}

        {events.length === 0 && !error && (
          <section className="wb-section">
            <p className="wb-hint">
              No evidence recorded yet. Every prompt, approval and turn result
              lands here automatically — work with an agent and come back.
            </p>
          </section>
        )}

        {!selectedCase && cases.length > 0 && (
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
                  onClick={() => void exportPack(c.caseId)}
                  title="Export a signed proof packet for this case"
                >
                  {exporting === c.caseId ? "signing…" : "export"}
                </button>
              </div>
            ))}
            <p className="wb-hint">
              <button
                className="wb-link"
                disabled={exporting !== null}
                onClick={() => void exportPack(undefined)}
                title="Export the full ledger as one signed packet"
              >
                {exporting === "__ledger__" ? "signing…" : "export full ledger"}
              </button>
            </p>
          </section>
        )}

        {selectedCase && (
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

        {lastExport && (
          <section className="wb-section">
            <div className="wb-section-title">last export</div>
            <p className="wb-hint">
              <code>{lastExport.path}</code>
              <br />
              {lastExport.eventCount} events
              {lastExport.stubCount > 0 && ` · ${lastExport.stubCount} stubs`}
              {lastExport.redactionCount > 0 &&
                ` · ${lastExport.redactionCount} redactions`}
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
