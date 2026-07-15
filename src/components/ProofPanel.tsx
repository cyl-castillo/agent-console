import { useEffect } from "react";

import { useProofStore, summarizeCases } from "../stores/proofStore";
import { useSessionStore } from "../stores/sessionStore";

/// F3 surface for the Testigo ledger: chain health, cases, one-click signed
/// packet export. The full per-turn timeline is F4; this panel stays the
/// export/verify home.
export function ProofPanel() {
  const project = useSessionStore((s) => s.project);
  const events = useProofStore((s) => s.events);
  const report = useProofStore((s) => s.report);
  const exporting = useProofStore((s) => s.exporting);
  const lastExport = useProofStore((s) => s.lastExport);
  const error = useProofStore((s) => s.error);
  const load = useProofStore((s) => s.load);
  const exportPack = useProofStore((s) => s.exportPack);

  useEffect(() => {
    if (project) void load(project.root);
  }, [project, load]);

  const cases = summarizeCases(events);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">proof</span>
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

        {cases.length > 0 && (
          <section className="wb-section">
            <div className="wb-section-title">cases</div>
            {cases.map((c) => (
              <div className="wb-row" key={c.caseId}>
                <span className="wb-row-main" title={c.caseId}>
                  <code>{c.caseId}</code>
                  <span className="wb-hint">
                    {" "}· {c.turns} turns · {c.approvals} approvals · {c.events} events
                  </span>
                </span>
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
