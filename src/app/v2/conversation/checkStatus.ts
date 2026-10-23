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
