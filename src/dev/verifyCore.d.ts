/// Types for the vendored reference verifier (.github/testigo/verify-core.mjs).
declare module "*/verify-core.mjs" {
  export function verifyPacket(
    pkt: unknown,
    enforce?: { predicateType?: string; exportedAt?: string },
  ): {
    valid: boolean;
    firstFailure: string | null;
    counts: { entries: number; recomputed: number; redacted: number; stubs: number };
  };
}
