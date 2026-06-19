import { useEffect, useMemo, useState } from "react";

import { useApprovalStore } from "../stores/approvalStore";
import { usePermissionsStore } from "../stores/permissionsStore";
import { useVoiceStore } from "../stores/voiceStore";
import { suggestRules, classify, buildRaw, isHardDenyAllow, assessCommand } from "../permissions/rules";
import type { RuleSuggestion, Scope } from "../permissions/types";
import type { ApprovalRequest } from "../types/domain";

function describe(req: ApprovalRequest): { primary: string; secondary?: string } {
  const inp = req.input ?? {};
  if (req.tool === "Bash") {
    return {
      primary: typeof inp.command === "string" ? inp.command : "(no command)",
      secondary: typeof inp.description === "string" ? inp.description : undefined,
    };
  }
  if (typeof inp.file_path === "string") {
    return { primary: inp.file_path, secondary: req.tool };
  }
  return { primary: JSON.stringify(inp).slice(0, 200), secondary: req.tool };
}

// --- Edit/Write diff preview ---

interface DiffLine {
  type: "add" | "del" | "ctx";
  text: string;
}

// LCS-based line diff so unchanged lines render as context instead of being
// counted as a delete+add pair. Keeps the +N/-N stat honest: editing one line
// inside a 20-line block shows +1/-1, not +20/-20.
function diffLines(oldStr: string, newStr: string): DiffLine[] {
  // Pure insertion / deletion: skip the LCS and the spurious empty-line pair
  // that "".split("\n") would otherwise introduce.
  if (oldStr === "") return newStr === "" ? [] : newStr.split("\n").map((l) => ({ type: "add", text: l }));
  if (newStr === "") return oldStr.split("\n").map((l) => ({ type: "del", text: l }));
  const a = oldStr.split("\n");
  const b = newStr.split("\n");
  const n = a.length;
  const m = b.length;

  // LCS length table.
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const lines: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      lines.push({ type: "ctx", text: a[i] });
      i++; j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      lines.push({ type: "del", text: a[i] });
      i++;
    } else {
      lines.push({ type: "add", text: b[j] });
      j++;
    }
  }
  while (i < n) lines.push({ type: "del", text: a[i++] });
  while (j < m) lines.push({ type: "add", text: b[j++] });
  return lines;
}

function editPreview(req: ApprovalRequest): DiffLine[] | null {
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

function DiffPreview({ lines }: { lines: DiffLine[] }) {
  const LIMIT = 50;
  const capped = lines.length > LIMIT;
  const visible = capped ? lines.slice(0, LIMIT) : lines;
  const adds = lines.filter((l) => l.type === "add").length;
  const dels = lines.filter((l) => l.type === "del").length;

  return (
    <div className="approval-diff">
      <div className="approval-diff-stat">
        {adds > 0 && <span className="diff-stat-add">+{adds}</span>}
        {dels > 0 && <span className="diff-stat-del">-{dels}</span>}
      </div>
      <pre className="approval-diff-body">
        {visible.map((line, i) => (
          <div key={i} className={`diff-${line.type}`}>
            {line.type === "add" ? "+" : line.type === "del" ? "-" : " "}
            {line.text}
          </div>
        ))}
        {capped && <div className="diff-ctx">  … {lines.length - LIMIT} more lines</div>}
      </pre>
    </div>
  );
}

export function ApprovalModal() {
  const queue = useApprovalStore((s) => s.queue);
  const req = queue[0] ?? null;
  const decide = useApprovalStore((s) => s.decide);
  const addRule = usePermissionsStore((s) => s.add);
  const voiceStage = useVoiceStore((s) => s.approvalStage);

  const [showAlways, setShowAlways] = useState(false);
  const [scope, setScope] = useState<Scope>("project");
  const [busy, setBusy] = useState(false);

  // Hoisted before the early-return guard so the keyboard handler can reference it.
  const cmdAssessment = useMemo(() => {
    if (!req || req.tool !== "Bash") return null;
    const cmd = typeof req.input?.command === "string" ? req.input.command : "";
    return assessCommand(cmd);
  }, [req]);

  useEffect(() => {
    setShowAlways(false);
    setScope("project");
    setBusy(false);
  }, [req?.id]);

  useEffect(() => {
    if (!req) return;
    const onKey = (e: KeyboardEvent) => {
      if (busy) return;
      if (e.key === "Escape") {
        decide(req.id, "ask", "user dismissed");
      } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !showAlways) {
        // Block the fast-approval shortcut when the command is dangerous — a
        // deliberate click is required so muscle memory can't bypass the badge.
        if (cmdAssessment?.level === "dangerous") return;
        e.preventDefault();
        setBusy(true);
        void decide(req.id, "allow", "approved once");
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [req, busy, showAlways, decide, cmdAssessment]);

  const suggestions: RuleSuggestion[] = useMemo(
    () => (req ? suggestRules(req, scope) : []),
    [req, scope],
  );

  if (!req) return null;
  const { primary, secondary } = describe(req);
  const diffLines = editPreview(req);

  return (
    <div className="modal-backdrop">
      <div className="modal approval-modal" onClick={(e) => e.stopPropagation()}>
        <div className="approval-head">
          <span className={`approval-tool tool-${req.tool.toLowerCase()}`}>{req.tool}</span>
          <span className="approval-cwd" title={req.cwd}>{shortenPath(req.cwd)}</span>
          {queue.length > 1 && (
            <span className="approval-queue-depth">{queue.length} queued</span>
          )}
        </div>

        {voiceStage === "speaking" && (
          <div className="approval-voice-hint">🔊 Announcing aloud…</div>
        )}
        {voiceStage === "listening" && (
          <div className="approval-voice-hint listening">
            🎙 Listening — say “sí” to approve or “no” to deny
          </div>
        )}

        <pre className="approval-primary">{primary}</pre>

        {cmdAssessment && (
          <div className="approval-bash-risk">
            <span className={`risk risk-${cmdAssessment.level}`}>
              {cmdAssessment.level}
            </span>
            <span className="approval-risk-hint">{cmdAssessment.reason}</span>
          </div>
        )}

        {diffLines && <DiffPreview lines={diffLines} />}

        {secondary && !diffLines && <div className="approval-secondary">{secondary}</div>}

        {!showAlways ? (
          <div className="approval-actions">
            <button
              className="btn btn-danger"
              disabled={busy}
              onClick={async () => { setBusy(true); await decide(req.id, "deny", "user denied"); }}
            >
              Deny
            </button>
            <button
              className="btn btn-secondary"
              disabled={busy}
              onClick={() => setShowAlways(true)}
            >
              Always…
            </button>
            <button
              className="btn btn-success"
              disabled={busy}
              title={cmdAssessment?.level === "dangerous" ? "Click required — keyboard shortcut disabled for dangerous commands" : "Ctrl+Enter"}
              onClick={async () => { setBusy(true); await decide(req.id, "allow", "approved once"); }}
            >
              Approve once
            </button>
          </div>
        ) : (
          <AlwaysPanel
            req={req}
            suggestions={suggestions}
            scope={scope}
            setScope={setScope}
            onCancel={() => setShowAlways(false)}
            onCommit={async (suggestion, effect) => {
              setBusy(true);
              const r = await addRule(suggestion.rule.scope, effect, suggestion.rule.raw);
              if (r) {
                await decide(req.id, effect === "deny" ? "deny" : "allow",
                  `rule saved: ${suggestion.rule.raw}`);
              } else {
                setBusy(false);
              }
            }}
          />
        )}
      </div>
    </div>
  );
}

interface AlwaysProps {
  req: ApprovalRequest;
  suggestions: RuleSuggestion[];
  scope: Scope;
  setScope: (s: Scope) => void;
  onCancel: () => void;
  onCommit: (s: RuleSuggestion, effect: "allow" | "deny") => void;
}

function AlwaysPanel({ req, suggestions, scope, setScope, onCancel, onCommit }: AlwaysProps) {
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [confirmText, setConfirmText] = useState("");
  const [denyMode, setDenyMode] = useState(false);

  const selected = suggestions[selectedIdx];

  useEffect(() => { setConfirmText(""); }, [selectedIdx, scope, denyMode]);

  if (!selected) return <div className="placeholder">No suggestions.</div>;

  const effect: "allow" | "deny" = denyMode ? "deny" : "allow";
  const live = { ...selected.rule, scope, effect, raw: buildRaw(selected.rule.tool, selected.rule.pattern) };
  const { risk, reason } = classify(live);
  const hd = effect === "allow" ? isHardDenyAllow(live.raw) : { hard: false } as const;
  const blocked = hd.hard;
  const strict = risk === "broad" || risk === "dangerous";
  const requiresTyping = !blocked && (
    risk === "dangerous" ||
    (effect === "allow" && scope === "global" && (live.tool === "Bash" || live.tool === "Write"))
  );

  return (
    <div className="approval-always">
      <div className="approval-row">
        <span className="label">Scope</span>
        <div className="seg">
          <button
            className={scope === "project" ? "on" : ""}
            onClick={() => setScope("project")}
          >project</button>
          <button
            className={scope === "global" ? "on" : ""}
            onClick={() => setScope("global")}
          >global</button>
        </div>
        <span className="label" style={{ marginLeft: 12 }}>Effect</span>
        <div className="seg">
          <button className={!denyMode ? "on" : ""} onClick={() => setDenyMode(false)}>allow</button>
          <button className={denyMode ? "on" : ""} onClick={() => setDenyMode(true)}>deny</button>
        </div>
      </div>

      <ul className="approval-suggestions">
        {suggestions.map((s, i) => (
          <li
            key={s.rule.raw}
            className={i === selectedIdx ? "selected" : ""}
            onClick={() => setSelectedIdx(i)}
          >
            <input type="radio" checked={i === selectedIdx} readOnly />
            <span className="sug-label">{s.label}</span>
          </li>
        ))}
      </ul>

      <div className="approval-preview">
        <div className="preview-line">
          <span className="label">Rule</span>
          <code>{live.raw}</code>
          <span className={`risk risk-${risk}`}>{risk}</span>
        </div>
        {reason && <div className="preview-reason">{reason}</div>}
        <div className="preview-line">
          <span className="label">Writes to</span>
          <code className="path">
            {scope === "project" ? ".claude/settings.json" : "~/.claude/settings.json"}
          </code>
        </div>
      </div>

      {blocked && (
        <div className="approval-blocked">
          <strong>Blocked</strong> — this pattern is in Agent Console's hard-deny list
          and cannot be saved as an allow rule. {hd.reason}
        </div>
      )}

      {requiresTyping && !blocked && (
        <div className="approval-typegate">
          <label>
            Type the rule exactly to confirm:&nbsp;
            <code>{live.raw}</code>
          </label>
          <input
            autoFocus
            value={confirmText}
            onChange={(e) => setConfirmText(e.target.value)}
            placeholder={live.raw}
          />
        </div>
      )}

      {strict && !requiresTyping && !blocked && (
        <div className="approval-confirm">
          <label>
            <input
              type="checkbox"
              checked={confirmText === "ok"}
              onChange={(e) => setConfirmText(e.target.checked ? "ok" : "")}
            />
            &nbsp;I understand this is a broad permission.
          </label>
        </div>
      )}

      <div className="approval-actions">
        <button className="btn btn-secondary" onClick={onCancel}>Back</button>
        <button
          className="btn btn-success"
          disabled={
            blocked ||
            (requiresTyping && confirmText !== live.raw) ||
            (!requiresTyping && strict && confirmText !== "ok")
          }
          onClick={() => onCommit({ ...selected, rule: live }, effect)}
        >
          Save rule {denyMode ? "(deny)" : "(allow)"} & {denyMode ? "deny" : "approve"} {req.tool}
        </button>
      </div>
    </div>
  );
}

function shortenPath(p: string): string {
  const home = "/home/";
  if (p.startsWith(home)) {
    const rest = p.slice(home.length);
    const slash = rest.indexOf("/");
    if (slash >= 0) return "~" + rest.slice(slash);
  }
  return p.length > 60 ? "…" + p.slice(-60) : p;
}
