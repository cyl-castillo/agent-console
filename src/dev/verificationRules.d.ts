// The PR gate's JS port of the verification rules, imported by the vitest
// vector test so the Rust rules, the gate and the verifier page agree.
declare module "*/verification-rules.mjs" {
  export type Claim = { phrase: string; lang: string; negated: boolean };
  export function recomputeVerification(
    events: { seq: number; kind: string; payload?: unknown }[],
  ): {
    status: string;
    coverage: string;
    claims: Claim[];
    checks: Record<string, number>;
  };
}
