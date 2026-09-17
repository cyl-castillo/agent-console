// What the approval modal shows for an Edit/Write/MultiEdit: a line diff of
// the agent's proposed change, plus the one-line description of any request.
//
// Pure module so the diff can be tested — and so its cost is bounded. The
// previous implementation built a full (n+1)×(m+1) LCS table on the UI
// thread: an Edit replacing a 5,000-line block allocated 25 million cells
// in the one moment the user is waiting on a modal to react. This one trims
// the common prefix/suffix first (most edits touch a few lines inside a
// large block), runs Myers' O((N+M)·D) diff on what's left, and past a hard
// edit-count cap gives up on alignment and shows the middle as deleted +
// added — an honest +N/−M rather than a frozen webview.

import type { ApprovalRequest } from "../types/domain";
import { toRelative } from "../permissions/rules";

export interface DiffLine {
  type: "add" | "del" | "ctx";
  text: string;
}

/// Edits (insertions + deletions) beyond which alignment is not attempted.
/// A real edit rarely needs more than a few hundred; a wholesale rewrite
/// reads fine as "everything went, everything came".
export const MAX_EDIT_DISTANCE = 1000;

/// LCS-quality line diff: unchanged lines render as context so the +N/−M
/// stat is honest (one changed line inside a 20-line block is +1/−1).
export function diffLines(oldStr: string, newStr: string): DiffLine[] {
  // Pure insertion / deletion: skip the diff and the spurious empty-line pair
  // that "".split("\n") would otherwise introduce.
  if (oldStr === "")
    return newStr === "" ? [] : newStr.split("\n").map((l) => ({ type: "add", text: l }));
  if (newStr === "") return oldStr.split("\n").map((l) => ({ type: "del", text: l }));
  const a = oldStr.split("\n");
  const b = newStr.split("\n");

  // Common prefix / suffix are context for free and shrink the problem to
  // the lines that actually differ.
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (
    tail < a.length - head &&
    tail < b.length - head &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  )
    tail++;

  const midA = a.slice(head, a.length - tail);
  const midB = b.slice(head, b.length - tail);
  const middle = myers(midA, midB, MAX_EDIT_DISTANCE) ?? [
    ...midA.map((text): DiffLine => ({ type: "del", text })),
    ...midB.map((text): DiffLine => ({ type: "add", text })),
  ];

  return [
    ...a.slice(0, head).map((text): DiffLine => ({ type: "ctx", text })),
    ...middle,
    ...a.slice(a.length - tail).map((text): DiffLine => ({ type: "ctx", text })),
  ];
}

/// Myers' greedy diff (An O(ND) Difference Algorithm, 1986). Returns null when
/// the edit distance exceeds `maxD` — the caller decides what to show then.
/// Memory is O(D²) for the trace (each step keeps only its own k-range), not
/// O(N·M): a 10,000-line pair with a 3-line change costs a few hundred ints.
function myers(a: string[], b: string[], maxD: number): DiffLine[] | null {
  const n = a.length;
  const m = b.length;
  if (n === 0) return b.map((text) => ({ type: "add", text }));
  if (m === 0) return a.map((text) => ({ type: "del", text }));
  const max = n + m;
  const limit = Math.min(maxD, max);
  // v[k + max] = furthest x reached on diagonal k. One array of size 2·max+1
  // is reused across steps; the trace keeps a copy of just [-d, d] per step.
  const v = new Int32Array(2 * max + 1);
  const trace: Int32Array[] = [];
  let found = -1;

  outer: for (let d = 0; d <= limit; d++) {
    trace.push(v.slice(max - d, max + d + 1));
    for (let k = -d; k <= d; k += 2) {
      let x: number;
      if (k === -d || (k !== d && v[max + k - 1] < v[max + k + 1])) x = v[max + k + 1];
      else x = v[max + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x++;
        y++;
      }
      v[max + k] = x;
      if (x >= n && y >= m) {
        found = d;
        break outer;
      }
    }
  }
  if (found < 0) return null;

  // Backtrack from (n, m) through the per-step snapshots.
  const out: DiffLine[] = [];
  let x = n;
  let y = m;
  for (let d = found; d > 0; d--) {
    // trace[d] was captured before step d ran, i.e. it is the state after
    // step d-1 — exactly what "where did step d come from" needs.
    const prev = trace[d];
    const at = (kk: number) => prev[kk + d];
    const k = x - y;
    let prevK: number;
    if (k === -d || (k !== d && at(k - 1) < at(k + 1))) prevK = k + 1;
    else prevK = k - 1;
    const prevX = at(prevK);
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      out.push({ type: "ctx", text: a[x - 1] });
      x--;
      y--;
    }
    if (x === prevX) out.push({ type: "add", text: b[prevY] });
    else out.push({ type: "del", text: a[prevX] });
    x = prevX;
    y = prevY;
  }
  while (x > 0 && y > 0) {
    out.push({ type: "ctx", text: a[x - 1] });
    x--;
    y--;
  }
  out.reverse();
  return normalizeRuns(out);
}

/// Within each maximal run of changed lines, list deletions before
/// insertions — the way every diff viewer renders a replacement. Myers'
/// backtrack interleaves them by diagonal, which is correct but reads as
/// "added then removed".
function normalizeRuns(lines: DiffLine[]): DiffLine[] {
  const out: DiffLine[] = [];
  let dels: DiffLine[] = [];
  let adds: DiffLine[] = [];
  const flush = () => {
    out.push(...dels, ...adds);
    dels = [];
    adds = [];
  };
  for (const l of lines) {
    if (l.type === "ctx") {
      flush();
      out.push(l);
    } else if (l.type === "del") dels.push(l);
    else adds.push(l);
  }
  flush();
  return out;
}

export function editPreview(req: ApprovalRequest): DiffLine[] | null {
  const inp = req.input ?? {};
  const tool = req.tool;

  if (tool === "Edit" || tool === "StrReplace") {
    const oldStr = typeof inp.old_string === "string" ? inp.old_string : null;
    const newStr = typeof inp.new_string === "string" ? inp.new_string : null;
    if (!oldStr && !newStr) return null;
    return diffLines(oldStr ?? "", newStr ?? "");
  }

  if (tool === "Write") {
    const content = typeof inp.content === "string" ? inp.content : null;
    if (!content) return null;
    return content.split("\n").map((l) => ({ type: "add" as const, text: l }));
  }

  if (tool === "MultiEdit") {
    const edits = Array.isArray(inp.edits)
      ? (inp.edits as Array<{ old_string?: string; new_string?: string }>)
      : [];
    const lines: DiffLine[] = [];
    edits.forEach((edit, i) => {
      if (i > 0) lines.push({ type: "ctx", text: "···" });
      lines.push(...diffLines(edit.old_string ?? "", edit.new_string ?? ""));
    });
    return lines.length > 0 ? lines : null;
  }

  return null;
}

export function describe(req: ApprovalRequest): { primary: string; secondary?: string } {
  const inp = req.input ?? {};
  if (req.tool === "Bash") {
    return {
      primary: typeof inp.command === "string" ? inp.command : "(no command)",
      secondary: typeof inp.description === "string" ? inp.description : undefined,
    };
  }
  if (typeof inp.file_path === "string") {
    // Show the path relative to the working dir so it's clear which file in
    // *this* repo is being touched (absolute agent-supplied paths are noisy).
    return { primary: toRelative(inp.file_path, req.cwd) ?? inp.file_path, secondary: req.tool };
  }
  return { primary: JSON.stringify(inp).slice(0, 200), secondary: req.tool };
}
