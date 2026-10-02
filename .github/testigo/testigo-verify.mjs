#!/usr/bin/env node
// P2 proof-on-PR gate: verify the proof packets a PR adds or updates under
// .testigo/proofs/, then check the claims that make a packet worth having
// on a PR at all:
//
//   1. The packet itself verifies (DSSE signature, subject digest, chain
//      linkage, content recompute) — via the vendored reference verifier.
//   2. Its `gitCommit` subject is a real commit in this PR's history: the
//      evidence binds to the code under review, not to the packet itself.
//   3. The PR's diff (minus .testigo/) is covered by the packet's recorded
//      work — turn_end.filesChanged (agent work) ∪ commit.files (human
//      commits the ledger witnessed). Uncovered files mean the PR carries
//      changes nobody recorded. When redaction or stubbing removed the
//      events that would prove coverage, the check reports "not checkable"
//      instead of pretending either way.
//
// A PR with no packet passes with a note: the packet is an upgrade, not a
// gate (same stance as the Open PR flow in-app). Trust anchoring of the
// signing key stays out-of-band (compare the reported keyid), like every
// other Testigo surface.
//
// Usage: node testigo-verify.mjs <base-sha> <head-sha>

import { execFileSync } from "node:child_process";
import fs from "node:fs";

import { verifyPacket } from "./verify-core.mjs";

const [base, head] = process.argv.slice(2);
if (!base || !head) {
  console.error("usage: testigo-verify.mjs <base-sha> <head-sha>");
  process.exit(2);
}

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

const lines = [];
const out = (s) => {
  console.log(s);
  lines.push(s);
};

const changed = git("diff", "--name-only", `${base}...${head}`)
  .split("\n")
  .filter(Boolean);
const packets = changed.filter(
  (f) =>
    f.startsWith(".testigo/proofs/") &&
    f.endsWith(".proofpack.json") &&
    fs.existsSync(f),
);

let failed = false;

if (packets.length === 0) {
  out("### Testigo");
  out("");
  out(
    "No proof packet in this PR — nothing to verify. A packet is an upgrade, " +
      'not a gate; attach one with "Attach proof" on Open PR in Agent Console.',
  );
} else {
  out("### Testigo proof");
  const covered = new Set();
  let coverageCheckable = true;
  let turns = 0;
  let approvals = 0;
  let checksPassed = 0;
  let checksFailed = 0;

  for (const file of packets) {
    const pkt = JSON.parse(fs.readFileSync(file, "utf8"));
    const r = verifyPacket(pkt);
    if (!r.valid) {
      failed = true;
      out(`- ✘ \`${file}\`: **INVALID** (${r.firstFailure})`);
      continue;
    }
    const st = JSON.parse(
      Buffer.from(pkt.envelope.payload, "base64").toString("utf8"),
    );
    const events = st.predicate?.events ?? [];
    const parsed = events
      .filter((e) => typeof e.line === "string")
      .map((e) => JSON.parse(e.line));

    for (const e of parsed) {
      // Manually redacted events lose their payload; if one of them was a
      // turn_end or commit, its files are gone and coverage can't be judged.
      const gone = e.payload?.redacted === "manual";
      if (e.kind === "turn_end") {
        turns++;
        if (gone || e.payload?.filesTruncated) coverageCheckable = false;
        for (const f of e.payload?.filesChanged ?? []) covered.add(f.path);
      } else if (e.kind === "commit") {
        if (gone || e.payload?.filesTruncated) coverageCheckable = false;
        for (const f of e.payload?.files ?? []) covered.add(f);
      } else if (e.kind === "approval_decision") {
        approvals++;
      } else if (e.kind === "check_run") {
        if (e.payload?.status === "passed") checksPassed++;
        else checksFailed++;
      }
    }
    // Work pruned to stubs can't prove coverage either.
    for (const e of events) {
      if (e.stub && (e.stub.kind === "turn_end" || e.stub.kind === "commit"))
        coverageCheckable = false;
    }

    out(
      `- ✔ \`${file}\`: signature + chain valid — ${r.counts.entries} entries, ` +
        `${r.counts.recomputed} recomputed, ${r.counts.redacted} redacted, ` +
        `${r.counts.stubs} stubs · keyid \`${r.keyId.slice(0, 16)}…\``,
    );

    // The git subject: the one line that binds evidence to the code.
    const gitSubj = (st.subject ?? []).find((s) => s.digest?.gitCommit);
    if (gitSubj) {
      const sha = gitSubj.digest.gitCommit;
      let ancestor = false;
      try {
        execFileSync("git", ["merge-base", "--is-ancestor", sha, head]);
        ancestor = true;
      } catch {
        ancestor = false;
      }
      if (ancestor) {
        out(`- ✔ git subject \`${sha.slice(0, 10)}\` is in this PR's history`);
      } else {
        failed = true;
        out(
          `- ✘ git subject \`${sha.slice(0, 10)}\` is **NOT** in this PR's ` +
            "history — the packet describes other code",
        );
      }
    } else {
      out("- ⚠ no git subject in the packet (pre-P2 export?) — evidence not bound to the code");
    }
  }

  if (!failed) {
    out("");
    out(
      `**${turns} turn${turns === 1 ? "" : "s"} · ${approvals} approval` +
        `${approvals === 1 ? "" : "s"} · ${checksPassed} check run` +
        `${checksPassed === 1 ? "" : "s"} passed` +
        (checksFailed ? ` (${checksFailed} failed)` : "") +
        "**",
    );
    const diffFiles = changed.filter((f) => !f.startsWith(".testigo/"));
    const uncovered = diffFiles.filter((f) => !covered.has(f));
    if (!coverageCheckable) {
      out(
        "- ⚠ diff coverage not checkable: redaction/stubbing removed turn or " +
          "commit events that would prove it",
      );
    } else if (uncovered.length === 0) {
      out(
        `- ✔ PR diff ⊆ recorded work (${diffFiles.length} file` +
          `${diffFiles.length === 1 ? "" : "s"}, all witnessed)`,
      );
    } else {
      failed = true;
      out(
        `- ✘ ${uncovered.length} file${uncovered.length === 1 ? "" : "s"} in ` +
          "the PR diff with no recorded turn or commit:",
      );
      for (const f of uncovered.slice(0, 20)) out(`  - \`${f}\``);
      if (uncovered.length > 20) out(`  - … ${uncovered.length - 20} more`);
    }
  }
}

if (process.env.GITHUB_STEP_SUMMARY) {
  fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, lines.join("\n") + "\n");
}
process.exit(failed ? 1 : 0);
