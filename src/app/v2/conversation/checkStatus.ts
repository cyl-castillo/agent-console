/** One vocabulary for a check's verdict across the thread and the Inspector:
 *  ✓ passed · ✕ failed · ∅ ran nothing (exit 0, zero tests) · ? no exit code
 *  seen · ∙ interrupted/other. `empty` and `unknown` are NOT green: a runner
 *  that ran nothing, or whose exit the engine never showed, proves nothing. */
export function checkGlyph(status: string): string {
  switch (status) {
    case "passed":
      return "✓";
    case "failed":
      return "✕";
    case "empty":
      return "∅";
    case "unknown":
      return "?";
    default:
      return "∙";
  }
}
export function checkTone(status: string): "ok" | "danger" | "warn" {
  return status === "passed" ? "ok" : status === "failed" ? "danger" : "warn";
}
/** The turn's verification verdict (F1c), one vocabulary for chip and
 *  Inspector: only `verified` is green. */
export function verificationLabel(status: string): string {
  switch (status) {
    case "verified":
      return "verified";
    case "unverified_claim":
      return "unverified claim";
    case "claim_contradicted":
      return "claim contradicted";
    case "stale_check":
      return "stale check";
    case "checked":
      return "checked";
    case "unwitnessed":
      return "unwitnessed";
    default:
      return "no claim";
  }
}
export function verificationGlyph(status: string): string {
  switch (status) {
    case "verified":
      return "✓";
    case "claim_contradicted":
      return "✕";
    case "unverified_claim":
    case "stale_check":
      return "⚠";
    case "checked":
      return "✓";
    default:
      return "∙";
  }
}
export function verificationTone(status: string): "ok" | "danger" | "warn" | "neutral" {
  switch (status) {
    case "verified":
    case "checked":
      return "ok";
    case "claim_contradicted":
      return "danger";
    case "unverified_claim":
    case "stale_check":
    case "unwitnessed":
      return "warn";
    default:
      return "neutral";
  }
}

export function checkTitle(status: string): string | undefined {
  switch (status) {
    case "empty":
      return "Exit 0, but the runner ran zero tests — this proves nothing";
    case "unknown":
      return "No exit code was visible to the console — not counted as passed";
    case "interrupted":
      return "Interrupted before it finished";
    default:
      return undefined;
  }
}
