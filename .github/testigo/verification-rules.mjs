// The verification contract, recomputed by a third party from a proof
// packet's own lines (COMPETENCIA-2026-10 F1c). Port of
// src-tauri/src/services/verification_rules.rs — the shared vectors in
// src-tauri/fixtures/verification/cases.json keep both (and the copy
// embedded in testigo-verifier.html between the same markers) in step.
//
// Inputs are ONE turn's events in seq order: {seq, kind, payload}. No I/O.
/* verification-rules:begin */
export const VERIFICATION_RULES_VERSION = 1;

const SHELL = new Set(["Bash", "bash", "shell", "Shell", "exec_command", "local_shell", "unified_exec", "container.exec", "run_terminal_cmd"]);
const EDIT = new Set(["Edit", "Write", "MultiEdit", "NotebookEdit", "apply_patch", "edit", "write", "patch", "multiedit", "str_replace_editor", "str_replace_based_edit_tool"]);
const canonicalTool = (t) => (SHELL.has(t) ? "shell" : EDIT.has(t) ? "edit" : "other");

const CLAIM_PHRASES = [
  ["all tests pass", "en"], ["tests pass", "en"], ["test passes", "en"], ["tests are passing", "en"],
  ["tests passing", "en"], ["test suite passes", "en"], ["tests succeed", "en"], ["all checks pass", "en"],
  ["checks pass", "en"], ["all green", "en"], ["ci is green", "en"], ["build passes", "en"],
  ["build succeeds", "en"], ["lint passes", "en"], ["lint is clean", "en"], ["typecheck passes", "en"],
  ["type checks pass", "en"],
  ["todos los tests pasan", "es"], ["los tests pasan", "es"], ["pasan los tests", "es"], ["las pruebas pasan", "es"],
  ["pasan las pruebas", "es"], ["pasan todos los tests", "es"], ["todo en verde", "es"], ["todo verde", "es"],
  ["el build pasa", "es"], ["compila sin errores", "es"], ["lint limpio", "es"], ["el lint pasa", "es"],
  ["typecheck pasa", "es"], ["sin errores de tipos", "es"], ["la suite pasa", "es"],
  ["todos os testes passam", "pt"], ["os testes passam", "pt"], ["testes passando", "pt"], ["tudo verde", "pt"],
  ["build passa", "pt"], ["lint limpo", "pt"],
];
const GUARDS = [
  "not ", "n't", "never", "without", "unable", "couldn", "didn", "did not", "do not", "no longer",
  "should", "would", "will", "once", "if ", "when ", "after you", "expect", "hopefully", "probably",
  "no ", "sin ", "nunca", "no pude", "no corr", "deber", "podr", "cuando", "una vez", "espero",
  "não", "nao ", "sem ", "nunca", "deveria", "deveriam", "poderia", "quando",
];
const RANK = { failed: 4, empty: 3, unknown: 2, interrupted: 1 };

export function foldTurn(events) {
  const f = { toolResults: 0, commands: 0, failedCommands: 0,
    checks: { passed: 0, failed: 0, empty: 0, unknown: 0, interrupted: 0 },
    gatesRan: {}, lastEditSeq: null, lastCheckSeq: null, lastGoodCheckSeq: null, summary: "", superseded: false };
  for (const e of events) {
    const p = e.payload ?? {};
    if (e.kind === "tool_result") {
      f.toolResults++;
      const canonical = typeof p.toolCanonical === "string" ? p.toolCanonical : canonicalTool(String(p.tool ?? ""));
      if (canonical === "shell") {
        f.commands++;
        if (p.failed === true || (typeof p.exitCode === "number" && p.exitCode !== 0)) f.failedCommands++;
      } else if (canonical === "edit") f.lastEditSeq = e.seq;
    } else if (e.kind === "check_run") {
      const status = typeof p.status === "string" ? p.status : "unknown";
      if (status in f.checks) f.checks[status]++; else f.checks.unknown++;
      f.lastCheckSeq = e.seq;
      if (status === "passed") f.lastGoodCheckSeq = e.seq;
      const kind = typeof p.kind === "string" ? p.kind : "test";
      if (!(kind in f.gatesRan) || (RANK[status] ?? 0) > (RANK[f.gatesRan[kind]] ?? 0)) f.gatesRan[kind] = status;
    } else if (e.kind === "turn_end") {
      f.summary = typeof p.summary === "string" ? p.summary : "";
      f.superseded = p.superseded === true;
    }
  }
  return f;
}
export const coverageOf = (f) => (f.superseded && f.toolResults === 0 ? "turn_boundary" : "per_tool");

export function findClaims(summary) {
  const out = [];
  const sentences = String(summary).toLowerCase().split(/[.\n;!?]/).map((s) => s.trim()).filter(Boolean);
  for (const sentence of sentences) {
    for (const [phrase, lang] of CLAIM_PHRASES) {
      const at = sentence.indexOf(phrase);
      if (at < 0) continue;
      if (out.some((c) => c.phrase !== phrase && c.phrase.includes(phrase) && sentence.includes(c.phrase))) continue;
      const before = sentence.slice(0, at);
      out.push({ phrase, lang, negated: GUARDS.some((g) => before.includes(g)) });
    }
  }
  return out;
}

export function verdictOf(f, claims) {
  const live = claims.filter((c) => !c.negated);
  const c = f.checks;
  const total = c.passed + c.failed + c.empty + c.unknown + c.interrupted;
  if (live.length === 0) return total > 0 ? "checked" : "no_claim";
  if (coverageOf(f) === "turn_boundary") return "unwitnessed";
  if (c.failed > 0 || c.empty > 0) return "claim_contradicted";
  if (f.lastGoodCheckSeq === null) return "unverified_claim";
  if (f.lastEditSeq !== null && f.lastEditSeq > f.lastGoodCheckSeq) return "stale_check";
  return "verified";
}

/// One turn's events → {status, coverage, claims, checks}.
export function recomputeVerification(events) {
  const f = foldTurn(events);
  const claims = findClaims(f.summary);
  return { status: verdictOf(f, claims), coverage: coverageOf(f), claims, checks: f.checks };
}
/* verification-rules:end */
