interface Props {
  diff: string;
  empty?: string;
}

interface Row {
  /// Original-side cell: text + classification.
  left:  { kind: "ctx" | "del" | "empty"; text: string; segs?: Seg[] };
  right: { kind: "ctx" | "add" | "empty"; text: string; segs?: Seg[] };
  oldNo: number | null;
  newNo: number | null;
}

interface Seg { text: string; changed: boolean }

interface Hunk {
  oldStart: number;
  newStart: number;
  rows: Row[];
}

export function SideBySideDiffViewer({ diff, empty = "No changes." }: Props) {
  if (!diff.trim()) return <div className="placeholder">{empty}</div>;

  const hunks = parseDiff(diff);
  if (hunks.length === 0) {
    return <div className="placeholder">No textual diff to show.</div>;
  }

  return (
    <div className="sbs-diff">
      {hunks.map((h, i) => (
        <div key={i} className="sbs-hunk">
          <div className="sbs-hunk-header">
            @@ −{h.oldStart} +{h.newStart} @@
          </div>
          <div className="sbs-table">
            {h.rows.map((row, j) => (
              <div key={j} className="sbs-row">
                <div className="sbs-gutter">{row.oldNo ?? ""}</div>
                <div className={`sbs-cell sbs-${row.left.kind}`}>{renderSegs(row.left.segs, row.left.text)}</div>
                <div className="sbs-gutter">{row.newNo ?? ""}</div>
                <div className={`sbs-cell sbs-${row.right.kind}`}>{renderSegs(row.right.segs, row.right.text)}</div>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function renderSegs(segs: Seg[] | undefined, fallback: string) {
  if (!segs) return <span>{fallback || " "}</span>;
  return (
    <span>
      {segs.map((s, i) =>
        s.changed
          ? <mark key={i} className="sbs-intra">{s.text}</mark>
          : <span key={i}>{s.text}</span>,
      )}
    </span>
  );
}

function parseDiff(diff: string): Hunk[] {
  const hunks: Hunk[] = [];
  const lines = diff.split("\n");
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (!m) { i++; continue; }
    const oldStart = parseInt(m[1], 10);
    const newStart = parseInt(m[2], 10);
    const hunkLines: string[] = [];
    i++;
    while (i < lines.length && !lines[i].startsWith("@@") && !lines[i].startsWith("diff --git")) {
      // Skip the trailing "\ No newline at end of file" marker.
      if (!lines[i].startsWith("\\")) hunkLines.push(lines[i]);
      i++;
    }
    hunks.push(buildHunkRows(oldStart, newStart, hunkLines));
  }
  return hunks;
}

function buildHunkRows(oldStart: number, newStart: number, lines: string[]): Hunk {
  const rows: Row[] = [];
  let oldNo = oldStart;
  let newNo = newStart;

  // Accumulate consecutive del/add blocks so we can pair them.
  let dels: string[] = [];
  let adds: string[] = [];

  function flushBlock() {
    if (dels.length === 0 && adds.length === 0) return;
    const pairs = Math.min(dels.length, adds.length);
    for (let k = 0; k < pairs; k++) {
      const [leftSegs, rightSegs] = intraLineDiff(dels[k], adds[k]);
      rows.push({
        left:  { kind: "del", text: dels[k], segs: leftSegs },
        right: { kind: "add", text: adds[k], segs: rightSegs },
        oldNo: oldNo++,
        newNo: newNo++,
      });
    }
    for (let k = pairs; k < dels.length; k++) {
      rows.push({
        left:  { kind: "del",   text: dels[k] },
        right: { kind: "empty", text: "" },
        oldNo: oldNo++,
        newNo: null,
      });
    }
    for (let k = pairs; k < adds.length; k++) {
      rows.push({
        left:  { kind: "empty", text: "" },
        right: { kind: "add",   text: adds[k] },
        oldNo: null,
        newNo: newNo++,
      });
    }
    dels = []; adds = [];
  }

  for (const raw of lines) {
    if (raw.startsWith("+")) {
      adds.push(raw.slice(1));
    } else if (raw.startsWith("-")) {
      dels.push(raw.slice(1));
    } else {
      flushBlock();
      // Context line (may start with a space, or be empty).
      const text = raw.startsWith(" ") ? raw.slice(1) : raw;
      rows.push({
        left:  { kind: "ctx", text },
        right: { kind: "ctx", text },
        oldNo: oldNo++,
        newNo: newNo++,
      });
    }
  }
  flushBlock();
  return { oldStart, newStart, rows };
}

/// Coarse intra-line diff: longest-common-substring split. Good enough to
/// highlight a renamed identifier or a tweaked argument. Avoids importing
/// a full diff library.
function intraLineDiff(a: string, b: string): [Seg[], Seg[]] {
  if (!a && !b) return [[{ text: "", changed: false }], [{ text: "", changed: false }]];
  if (!a || !b) {
    return [
      [{ text: a, changed: a.length > 0 }],
      [{ text: b, changed: b.length > 0 }],
    ];
  }
  // Find longest common substring via DP. Cap input size to avoid n*m blowup
  // on huge lines — fall back to "whole line changed" if either is too long.
  const MAX = 400;
  if (a.length > MAX || b.length > MAX) {
    return [[{ text: a, changed: true }], [{ text: b, changed: true }]];
  }
  const dp = Array.from({ length: a.length + 1 }, () => new Uint16Array(b.length + 1));
  let bestLen = 0, ai = 0, bi = 0;
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      if (a[i - 1] === b[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1] + 1;
        if (dp[i][j] > bestLen) {
          bestLen = dp[i][j];
          ai = i - bestLen;
          bi = j - bestLen;
        }
      }
    }
  }
  // No meaningful common substring → both fully changed.
  if (bestLen < 2) {
    return [[{ text: a, changed: true }], [{ text: b, changed: true }]];
  }
  const aBefore = a.slice(0, ai);
  const aCommon = a.slice(ai, ai + bestLen);
  const aAfter  = a.slice(ai + bestLen);
  const bBefore = b.slice(0, bi);
  const bAfter  = b.slice(bi + bestLen);

  const left: Seg[] = [];
  if (aBefore) left.push({ text: aBefore, changed: true });
  left.push({ text: aCommon, changed: false });
  if (aAfter)  left.push({ text: aAfter, changed: true });

  const right: Seg[] = [];
  if (bBefore) right.push({ text: bBefore, changed: true });
  right.push({ text: aCommon, changed: false });
  if (bAfter)  right.push({ text: bAfter, changed: true });

  return [left, right];
}
