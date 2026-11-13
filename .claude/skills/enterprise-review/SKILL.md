---
name: enterprise-review
description: Run the full pre-release review of Agent Console (security, reliability, enterprise readiness, UX/a11y, performance) with independent verification of every serious finding, then fix what survives in small PRs. Use before a release aimed at teams, after a large batch of merges, or when asked to "review everything".
---

The review is a loop with a gate, not a prompt: reviewers produce claims,
a different agent tries to refute each one, and only confirmed claims are
fixed. A finding nobody could confirm is recorded as unproven and left alone.

## 1. Fan out (parallel, read-only)

Launch one agent per file in `.claude/agents/` except `finding-verifier`:
`security-reviewer`, `reliability-reviewer`, `enterprise-readiness-reviewer`,
`ux-a11y-reviewer`, `performance-reviewer`. Give each only: the repo path,
"follow your agent definition", and the commit under review. Do not hand them
your suspicions — an agent given a hypothesis returns it confirmed.

## 2. Triage

Merge the reports. Drop duplicates (keep the one with the better evidence),
anything a CI gate already enforces, and anything already tracked in an open
PR (`gh pr list`). Order by severity, then by cost of the fix.

## 3. Verify (verifier ≠ producer)

Every S1 and S2 goes to a fresh `finding-verifier` — one finding per agent,
given the claim and location, not the reporter's reasoning. Read the code
yourself for anything you intend to fix. REFUTED and unproven findings are
listed in the final report with the reason; they are not fixed.

## 4. Fix

- One confirmed finding (or one tight family) per branch and PR.
- Write the regression test first when the defect is testable; watch it fail.
- Run the gates in `CLAUDE.md` for what you touched; report the real output.
- Commits signed off, staged by name. Open the PR; never merge or release.
- A fix that changes GUI behaviour says "needs a human GUI check" and lists
  what to click.
- A fix that would change a public contract (action ids, ledger format,
  pairing protocol, persisted file shapes) is proposed, not applied.

## 5. Report

One table: finding, severity, verdict, PR or reason not fixed. Then what was
not covered. No finding is described as fixed before its PR's CI is green.

## Measuring the harness itself

`CLAUDE.md`, `AGENTS.md`, `.claude/agents/` and this skill are what the
Harness Lab (Learning → Harness lab) is allowed to edit. Its held-out tasks
live in the console's data dir, outside the repo, and are never shown to the
agent. If a review keeps missing a class of defect, add a held-out task that
would have caught it rather than a paragraph here.
