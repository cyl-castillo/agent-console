import { describe, expect, it } from "vitest";

import { verifyPacket } from "../../.github/testigo/verify-core.mjs";
import mixedPacket from "../../src-tauri/fixtures/ledgers/mixed.proofpack.json?raw";
import v1Packet from "../../src-tauri/fixtures/ledgers/v1.proofpack.json?raw";

type Entry = { line?: string; stub?: unknown };

const linesOf = (raw: string) => {
  const pkt = JSON.parse(raw);
  const st = JSON.parse(
    new TextDecoder().decode(Uint8Array.from(atob(pkt.envelope.payload), (c) => c.charCodeAt(0))),
  );
  return (st.predicate.events as Entry[]).flatMap((e) => (e.line ? [JSON.parse(e.line)] : []));
};

/// The packets are frozen bytes (src-tauri/fixtures/ledgers). The verifier is
/// the vendored reference one, which knows nothing about `schemaVersion`: it
/// recomputes each hash over the raw line, so a line that declares its format
/// verifies exactly like one that does not.
describe("the reference verifier and the ledger's line formats", () => {
  it("verifies a packet of lines written before schemaVersion existed", () => {
    const r = verifyPacket(JSON.parse(v1Packet));
    expect(r).toMatchObject({ valid: true, firstFailure: null });
    expect(r.counts.recomputed).toBeGreaterThan(0);
    expect(linesOf(v1Packet).every((l) => l.schemaVersion === undefined)).toBe(true);
  });

  it("verifies a packet that mixes both line formats", () => {
    const r = verifyPacket(JSON.parse(mixedPacket));
    expect(r).toMatchObject({ valid: true, firstFailure: null });
    const versions = linesOf(mixedPacket).map((l) => l.schemaVersion ?? 1);
    expect(new Set(versions)).toEqual(new Set([1, 2]));
  });
});
