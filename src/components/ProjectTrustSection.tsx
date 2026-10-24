import { surfaceSummary } from "../lib/trust";
import { useSessionStore } from "../stores/sessionStore";
import { useTrustStore } from "../stores/trustStore";
import { Chip } from "../ui/Chip";

/// F4: the folder's agent surface and the human's word on it, in the Trust
/// tab — the decision lives in the Inbox; here it can be revisited.
export function ProjectTrustSection() {
  const status = useTrustStore((s) => s.status);
  const loading = useTrustStore((s) => s.loading);
  const decide = useTrustStore((s) => s.decide);
  const load = useTrustStore((s) => s.load);
  const root = useSessionStore((s) => s.project?.root);
  if (!root) return null;
  const tone =
    status?.state === "trusted"
      ? "ok"
      : status?.state === "declined"
        ? "danger"
        : status
          ? "warn"
          : "neutral";
  return (
    <section className="wb-section" data-testid="project-trust">
      <div className="wb-section-title">
        workspace trust
        {status && (
          <Chip tone={tone} mono>
            {status.state}
          </Chip>
        )}
      </div>
      <p className="wb-hint">
        {status
          ? `${surfaceSummary(status)} · digest ${status.surface.digest.slice(0, 12)}…`
          : loading
            ? "reading the folder…"
            : "not read yet"}
        {status?.decision && (
          <>
            {" "}
            · last word: {status.decision.decision} on{" "}
            {new Date(status.decision.ts).toLocaleDateString()}
          </>
        )}
      </p>
      <div className="flex flex-wrap gap-2">
        {status?.state !== "trusted" && (
          <button className="wb-cta" onClick={() => void decide("trusted")}>
            trust this folder
          </button>
        )}
        {status?.state !== "declined" && (
          <button className="wb-cta" onClick={() => void decide("declined")}>
            don't trust
          </button>
        )}
        <button className="wb-cta" onClick={() => void load(root)}>
          re-check
        </button>
      </div>
    </section>
  );
}
