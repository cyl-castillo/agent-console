#!/usr/bin/env node
// Standalone verifier for an IETF Agent Audit Trail export
// (draft-sharif-agent-audit-trail-08), Section 6.3 procedure. No
// dependencies, no Agent Console: `node aat-verify.mjs <file.jsonl>...`.
// Exit 0 when every chain verifies, 1 otherwise.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// RFC 8785 (JCS): sorted keys by UTF-16 code units, ES6 number formatting
// (which JSON.stringify already produces), no whitespace.
export function canonicalize(v) {
  if (v === null || typeof v !== "object") return JSON.stringify(v);
  if (Array.isArray(v)) return `[${v.map(canonicalize).join(",")}]`;
  const keys = Object.keys(v).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalize(v[k])}`).join(",")}}`;
}

export function recordHash(record) {
  const { signature: _s, signature_classical: _c, batch: _b, ...rest } = record;
  return createHash("sha256").update(canonicalize(rest), "utf8").digest("hex");
}

const REQUIRED = [
  "record_id",
  "timestamp",
  "agent_id",
  "principal",
  "agent_version",
  "session_id",
  "action_type",
  "action_detail",
  "outcome",
  "trust_level",
  "record_phase",
];

/** Verify one chain given its JSONL text. */
export function verifyJsonl(text) {
  const problems = [];
  const lines = text.split("\n").filter((l) => l.trim() !== "");
  let prev = null;
  let brokenAt = null;
  let lastTs = "";
  lines.forEach((line, n) => {
    const rec = JSON.parse(line);
    for (const k of REQUIRED) if (!(k in rec)) problems.push(`record ${n}: missing ${k}`);
    if ("sequence_number" in rec && rec.sequence_number !== n)
      problems.push(`record ${n}: sequence_number ${rec.sequence_number} is not gap-free`);
    if (prev === null) {
      if (rec.parent_record_id !== null || rec.prev_hash !== null)
        problems.push("genesis record must have null parent_record_id and prev_hash");
      if (rec.action_type !== "lifecycle" || rec.action_detail?.event !== "session_start")
        problems.push("genesis record must be lifecycle/session_start");
    } else {
      if (rec.prev_hash !== recordHash(prev)) {
        problems.push(`record ${n}: prev_hash does not match the previous record`);
        if (brokenAt === null) brokenAt = n;
      }
      if (rec.parent_record_id !== prev.record_id)
        problems.push(`record ${n}: parent_record_id does not name the previous record`);
    }
    if (rec.timestamp < lastTs) problems.push(`record ${n}: timestamp goes backwards`);
    lastTs = rec.timestamp;
    prev = rec;
  });
  const tailClosed =
    prev !== null &&
    prev.action_type === "lifecycle" &&
    prev.action_detail?.event === "session_end";
  if (!tailClosed) problems.push("tail unverifiable: the last record is not a session_end close");
  if (tailClosed) {
    // session_hash = SHA-256 over the raw prev_hash digests of every record.
    const h = createHash("sha256");
    for (const l of lines) {
      const r = JSON.parse(l);
      if (typeof r.prev_hash === "string") h.update(Buffer.from(r.prev_hash, "hex"));
    }
    if (prev.action_detail.session_hash !== h.digest("hex"))
      problems.push("close record: session_hash does not match the chain");
    if (prev.action_detail.record_count !== lines.length)
      problems.push(
        `close record: record_count ${prev.action_detail.record_count} != ${lines.length}`,
      );
  }
  return { ok: problems.length === 0, records: lines.length, brokenAt, tailClosed, problems };
}

function main(files) {
  if (files.length === 0) {
    console.error("usage: node aat-verify.mjs <session.jsonl>...");
    process.exit(2);
  }
  let allOk = true;
  for (const f of files) {
    const r = verifyJsonl(readFileSync(f, "utf8"));
    allOk &&= r.ok;
    console.log(
      `${r.ok ? "OK " : "BAD"} ${f}: ${r.records} records${r.tailClosed ? ", closed" : ""}`,
    );
    for (const p of r.problems) console.log(`    - ${p}`);
  }
  process.exit(allOk ? 0 : 1);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1])
  main(process.argv.slice(2));
