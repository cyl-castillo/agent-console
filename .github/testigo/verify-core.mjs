#!/usr/bin/env node
// VENDORED verbatim (plus this note) from cyl-castillo/testigo main
// `conformance/verify.mjs` @ 13dda5b1c0d73c744030803f455d71ff3aafa893 —
// same resync discipline as src-tauri/resources/testigo-verifier.html:
// never edit here, resync from upstream and `cmp` against it.
//
// Reference verifier for Testigo proof packets (spec §2.4 + §2.5), used as
// the conformance-suite runner: it verifies every vector in vectors/ and
// compares the outcome against manifest.json expectations.
//
// It is deliberately written from the spec, not shared with generate.mjs —
// two independent code paths (plus the HTML verifier and the Rust reference
// implementation) have to agree on every vector for the suite to pass.
//
// Usage:
//   node verify.mjs                     # run the suite against vectors/
//   node verify.mjs some.proofpack.json # verify one packet, print the result

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const sha256hex = (buf) => crypto.createHash("sha256").update(buf).digest("hex");

/// Verify one packet per §2.4. Returns:
///   { valid, firstFailure, counts: {entries, recomputed, redacted, stubs},
///     timestamp: "none" | "declared" | "mismatch", keyId }
/// firstFailure ∈ format | keyid | signature | payload | predicateType |
///   exportedAt | digest | linkage | contentHash | redactionCount |
///   processContext | timestamps
///
/// `enforce` turns this into a checker of a SPECIFIC predicate (a manifest's
/// `enforce` block): { predicateType } requires an exact type URI (spec §5 —
/// verifiers reject predicate types they don't implement); { exportedAt:
/// "rfc3339" } requires the session-chain time convention. The migration-
/// guard vectors exist to catch checkers that skip these.
export function verifyPacket(pkt, enforce = {}) {
  const fail = (code) => ({ valid: false, firstFailure: code });

  // 1. Format.
  if (pkt.format !== "testigo-proofpack/v0.1") return fail("format");

  // 2. keyid = sha256 of the embedded raw public key.
  const pubRaw = Buffer.from(pkt.publicKey ?? "", "base64");
  const keyId = sha256hex(pubRaw);
  const sigEntry = pkt.envelope?.signatures?.[0] ?? {};
  if (sigEntry.keyid !== keyId) return fail("keyid");

  // 3. Ed25519 over the DSSE pre-authentication encoding.
  const payload = Buffer.from(pkt.envelope.payload ?? "", "base64");
  const type = pkt.envelope.payloadType ?? "";
  const paeBuf = Buffer.concat([
    Buffer.from(`DSSEv1 ${type.length} ${type} ${payload.length} `, "utf8"),
    payload,
  ]);
  const spki = Buffer.concat([
    Buffer.from("302a300506032b6570032100", "hex"),
    pubRaw,
  ]);
  let sigOk = false;
  try {
    const key = crypto.createPublicKey({ key: spki, format: "der", type: "spki" });
    sigOk = crypto.verify(null, paeBuf, key, Buffer.from(sigEntry.sig ?? "", "base64"));
  } catch {
    sigOk = false;
  }
  if (!sigOk) return fail("signature");

  // 4. Statement parses; subject digest matches the packed events.
  let st;
  try {
    st = JSON.parse(payload.toString("utf8"));
  } catch {
    return fail("payload");
  }
  if (enforce.predicateType && st.predicateType !== enforce.predicateType)
    return fail("predicateType");
  if (
    enforce.exportedAt === "rfc3339" &&
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/.test(st.predicate?.exportedAt ?? "")
  )
    return fail("exportedAt");
  const events = st.predicate?.events ?? [];
  const want = st.subject?.[0]?.digest?.sha256 ?? "";
  if (sha256hex(Buffer.from(JSON.stringify(events), "utf8")) !== want) return fail("digest");

  // 5 + 6. Linkage across every entry; content recompute for clean lines.
  let prev = st.predicate?.range?.prevHashBefore ?? "genesis";
  const counts = { entries: events.length, recomputed: 0, redacted: 0, stubs: 0 };
  for (const e of events) {
    let prevHash, hash;
    if (typeof e.line === "string") {
      let v;
      try {
        v = JSON.parse(e.line);
      } catch {
        return fail("linkage");
      }
      ({ prevHash, hash } = v);
      if (e.redacted) counts.redacted++;
    } else if (e.stub) {
      ({ prevHash, hash } = e.stub);
      counts.stubs++;
    } else {
      return fail("linkage");
    }
    if (prevHash !== prev) return fail("linkage");
    if (typeof e.line === "string" && !e.redacted) {
      // A member after hash violates §1.5. Never discard a suffix while
      // recomputing: all original bytes except the hash value are covered.
      const field = /("hash"\s*:\s*")([0-9a-f]{64})("\s*}\s*)$/.exec(e.line);
      if (!field || field[2] !== hash) return fail("contentHash");
      const unhashed = e.line.slice(0, field.index) + field[1] + field[3];
      const recomputed = sha256hex(Buffer.from(unhashed, "utf8"));
      if (recomputed !== hash) return fail("contentHash");
      counts.recomputed++;
    }
    prev = hash;
  }

  // 6b. Declared redaction count must match the entries (§2.3: stubs are
  // pruning, not redaction — a signed-over miscount misrepresents what was
  // withheld).
  if ((st.predicate?.redactionCount ?? 0) !== counts.redacted) return fail("redactionCount");

  // 6c. Process context (§2.6): every field optional; present ones must be
  // well-formed, and the declared session window must equal what the hashed
  // lines carry (the one claim among them a verifier can actually check).
  const pc = checkProcessContext(st.predicate ?? {}, events);
  if (pc) return fail(pc);

  // 8. Timestamp (§2.5): informative — declared or mismatching, never "verified".
  let timestamp = "none";
  const tsp = pkt.timestamp;
  if (tsp && tsp.type === "rfc3161") {
    const sigDigest = sha256hex(Buffer.from(sigEntry.sig, "base64"));
    let token = null;
    try {
      token = Buffer.from(tsp.token ?? "", "base64");
    } catch {
      token = null;
    }
    const ok =
      sigDigest === String(tsp.messageImprint ?? "").toLowerCase() &&
      token !== null &&
      token.includes(Buffer.from(sigDigest, "hex"));
    timestamp = ok ? "declared" : "mismatch";
  }

  return { valid: true, firstFailure: null, counts, timestamp, keyId };
}

const RFC3339_Z = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/;

/// §2.6 checks. Returns a failure code or null. `processContext` = a present
/// field is malformed; `timestamps` = the declared window does not equal the
/// first/last non-stub line's `ts` (or only one bound is declared).
export function checkProcessContext(pred, events) {
  const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
  const nonEmpty = (v) => typeof v === "string" && v.length > 0;
  if (pred.provider !== undefined) {
    const p = pred.provider;
    if (!isObj(p) || !isObj(p.harness) || !nonEmpty(p.harness.name) || !nonEmpty(p.harness.version))
      return "processContext";
    if (p.agent !== undefined && !isObj(p.agent)) return "processContext";
    if (p.languageModels !== undefined && !(Array.isArray(p.languageModels) && p.languageModels.every(isObj)))
      return "processContext";
  }
  if (pred.contextArtifacts !== undefined) {
    if (!Array.isArray(pred.contextArtifacts)) return "processContext";
    for (const a of pred.contextArtifacts) {
      if (!isObj(a)) return "processContext";
      if (nonEmpty(a.uri) === nonEmpty(a.data)) return "processContext"; // exactly one of uri / data
      if (a.digest !== undefined && !(isObj(a.digest) && /^[0-9a-f]{64}$/.test(a.digest.sha256 ?? "")))
        return "processContext";
      if (a.tags !== undefined && !(Array.isArray(a.tags) && a.tags.every(nonEmpty))) return "processContext";
    }
  }
  if (pred.owner !== undefined && !nonEmpty(pred.owner)) return "processContext";
  const hasStart = pred.startTimestamp !== undefined;
  const hasEnd = pred.endTimestamp !== undefined;
  if (hasStart !== hasEnd) return "timestamps";
  if (hasStart) {
    if (!RFC3339_Z.test(pred.startTimestamp) || !RFC3339_Z.test(pred.endTimestamp)) return "timestamps";
    const ts = events.filter((e) => typeof e.line === "string").map((e) => JSON.parse(e.line).ts);
    if (!ts.length) return "timestamps";
    if (Date.parse(pred.startTimestamp) !== ts[0] || Date.parse(pred.endTimestamp) !== ts.at(-1)) return "timestamps";
  }
  return null;
}

// ---- runner (only when executed directly — verifyPacket stays importable) ---

function runManifest(dir, label) {
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, "manifest.json"), "utf8"));
  let failures = 0;
  for (const v of manifest.vectors) {
    const got = verifyPacket(JSON.parse(fs.readFileSync(path.join(dir, v.file), "utf8")), manifest.enforce ?? {});
    const problems = [];
    if (got.valid !== v.expect.valid) problems.push(`valid: got ${got.valid}, want ${v.expect.valid}`);
    if (!v.expect.valid && got.firstFailure !== v.expect.firstFailure)
      problems.push(`firstFailure: got ${got.firstFailure}, want ${v.expect.firstFailure}`);
    if (v.expect.counts)
      for (const [k, want] of Object.entries(v.expect.counts))
        if (got.counts?.[k] !== want) problems.push(`counts.${k}: got ${got.counts?.[k]}, want ${want}`);
    if (v.expect.timestamp && got.timestamp !== v.expect.timestamp)
      problems.push(`timestamp: got ${got.timestamp}, want ${v.expect.timestamp}`);
    if (problems.length) {
      failures++;
      console.log(`FAIL  ${v.file}\n      ${problems.join("\n      ")}`);
    } else {
      console.log(`ok    ${v.file}`);
    }
  }
  console.log(failures ? `\n${failures} ${label} vector(s) failed\n` : `\nall ${manifest.vectors.length} ${label} vectors pass\n`);
  return failures;
}

function runSuite() {
  const here = path.dirname(new URL(import.meta.url).pathname);
  const arg = process.argv[2];

  if (arg) {
    const result = verifyPacket(JSON.parse(fs.readFileSync(arg, "utf8")));
    console.log(JSON.stringify(result, null, 2));
    process.exit(result.valid ? 0 : 1);
  }

  let failures = runManifest(path.join(here, "vectors"), "conformance");
  // The session-chain draft subset (predicate/vectors) exercises the same
  // rules under the proposed in-toto predicate conventions.
  const scDir = path.join(here, "..", "predicate", "vectors");
  if (fs.existsSync(path.join(scDir, "manifest.json"))) failures += runManifest(scDir, "session-chain");
  process.exit(failures ? 1 : 0);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) runSuite();
