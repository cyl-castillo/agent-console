import { Fragment, useEffect, useMemo, useState } from "react";

import { useApprovalStore } from "../stores/approvalStore";
import { usePermissionsStore } from "../stores/permissionsStore";
import { useVoiceStore } from "../stores/voiceStore";
import { suggestRules, classify, buildRaw, isHardDenyAllow, assessCommand, toRelative } from "../permissions/rules";
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
    // Show the path relative to the working dir so it's clear which file in
    // *this* repo is being touched (absolute agent-supplied paths are noisy).
    return { primary: toRelative(inp.file_path, req.cwd) ?? inp.file_path, secondary: req.tool };
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
  const [expanded, setExpanded] = useState(false);
  const overflow = lines.length > LIMIT;
  const capped = overflow && !expanded;
  const visible = capped ? lines.slice(0, LIMIT) : lines;
  const adds = lines.filter((l) => l.type === "add").length;
  const dels = lines.filter((l) => l.type === "del").length;

  return (
    <div className="approval-diff">
      <div className="approval-diff-stat">
        {adds > 0 && <span className="diff-stat-add">+{adds}</span>}
        {dels > 0 && <span className="diff-stat-del">-{dels}</span>}
      </div>
      <pre className={`approval-diff-body${expanded ? " expanded" : ""}`}>
        {visible.map((line, i) => (
          <div key={i} className={`diff-${line.type}`}>
            {line.type === "add" ? "+" : line.type === "del" ? "-" : " "}
            {line.text}
          </div>
        ))}
      </pre>
      {overflow && (
        <button
          type="button"
          className="btn btn-link approval-diff-toggle"
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? "Show less" : `Show all ${lines.length} lines (+${adds}/-${dels})`}
        </button>
      )}
    </div>
  );
}

/// The agent is BLOCKED while this modal waits — and the hook gives up after
/// its timeout (falling back to the in-terminal prompt). Both facts used to
/// be invisible: users watched a "slow agent" that was actually waiting on
/// them, then wondered why the terminal asked again. Count it down honestly.
function ApprovalCountdown({ req }: { req: { ts: number; timeoutMs?: number } }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  const deadline = req.ts + (req.timeoutMs ?? 90_000);
  const left = Math.max(0, Math.ceil((deadline - now) / 1000));
  if (left === 0) {
    return (
      <span className="approval-countdown expired" title="The hook timed out; the agent fell back to its own terminal prompt.">
        expired — answer in the terminal
      </span>
    );
  }
  return (
    <span
      className={`approval-countdown${left <= 15 ? " urgent" : ""}`}
      title="The agent is blocked waiting for you. After this, the hook gives up and the agent falls back to its in-terminal prompt."
    >
      agent waiting · {left}s
    </span>
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

  // After the LAST pending decision, hand focus back to the terminal so the
  // keyboard flow continues where the interruption started.
  const decideAndRefocus = (id: string, decision: "allow" | "deny" | "ask", reason: string) => {
    const wasLast = queue.length <= 1;
    void decide(id, decision, reason).then(() => {
      if (wasLast) window.dispatchEvent(new CustomEvent("ac:focus-terminal"));
    });
  };

  useEffect(() => {
    if (!req) return;
    const onKey = (e: KeyboardEvent) => {
      if (busy) return;
      if (e.key === "Escape") {
        decideAndRefocus(req.id, "ask", "user dismissed");
      } else if ((e.key === "d" || e.key === "D") && (e.ctrlKey || e.metaKey) && !showAlways) {
        // Denying is always safe — no danger-gate needed on this shortcut.
        e.preventDefault();
        setBusy(true);
        decideAndRefocus(req.id, "deny", "user denied");
      } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !showAlways) {
        // Block the fast-approval shortcut when the command is dangerous — a
        // deliberate click is required so muscle memory can't bypass the badge.
        if (cmdAssessment?.level === "dangerous") return;
        e.preventDefault();
        setBusy(true);
        decideAndRefocus(req.id, "allow", "approved once");
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
    // decideAndRefocus is stable enough per render; queue.length feeds wasLast.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [req, busy, showAlways, decide, cmdAssessment, queue.length]);

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
          <ApprovalCountdown req={req} />
          {queue.length > 1 && (
            <span className="approval-queue-depth">1 of {queue.length}</span>
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
              onClick={() => { setBusy(true); decideAndRefocus(req.id, "deny", "user denied"); }}
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
              onClick={() => { setBusy(true); decideAndRefocus(req.id, "allow", "approved once"); }}
            >
              Approve once
            </button>
          </div>
        ) : null}

        {!showAlways && (
          <div className="approval-hint">
            <strong>Deny</strong> tells the agent no — it keeps going without running this.
            <span className="approval-hint-keys"><kbd>Esc</kbd> dismiss · <kbd>Ctrl</kbd>+<kbd>D</kbd> deny · <kbd>Ctrl</kbd>+<kbd>Enter</kbd> approve</span>
          </div>
        )}

        {showAlways && (
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

  // ↑/↓ cycle through the rule suggestions — the list was click-only before,
  // stranding keyboard users. Arrows are harmless in the single-line confirm
  // input, so no focus carve-out is needed.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      e.preventDefault();
      setSelectedIdx((i) => {
        const next = e.key === "ArrowDown" ? i + 1 : i - 1;
        return Math.max(0, Math.min(suggestions.length - 1, next));
      });
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [suggestions.length]);

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
        {suggestions.map((s, i) => {
          // Whole-tool grants ("ANY Bash", "ANY Edit") are far broader than the
          // exact/prefix rules they sit beside. Separate them under a divider and
          // mark them so they don't read as a peer of the safe exact-command rule.
          const broad = s.rule.pattern === null;
          const firstBroad = broad && suggestions.findIndex((x) => x.rule.pattern === null) === i;
          return (
            <Fragment key={s.rule.raw}>
              {firstBroad && (
                <li className="approval-sug-divider" aria-hidden="true">
                  Broader — applies to more than this one action
                </li>
              )}
              <li
                className={`${i === selectedIdx ? "selected" : ""}${broad ? " broad" : ""}`}
                onClick={() => setSelectedIdx(i)}
              >
                <input type="radio" checked={i === selectedIdx} readOnly />
                <span className="sug-label">{s.label}</span>
                {broad && <span className="sug-warn">broad</span>}
              </li>
            </Fragment>
          );
        })}
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
