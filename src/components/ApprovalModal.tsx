import { useEffect, useMemo, useState } from "react";

import { useApprovalStore } from "../stores/approvalStore";
import { usePermissionsStore } from "../stores/permissionsStore";
import { suggestRules, classify, buildRaw, isHardDenyAllow } from "../permissions/rules";
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

export function ApprovalModal() {
  const req = useApprovalStore((s) => s.queue[0] ?? null);
  const decide = useApprovalStore((s) => s.decide);
  const addRule = usePermissionsStore((s) => s.add);

  const [showAlways, setShowAlways] = useState(false);
  const [scope, setScope] = useState<Scope>("project");
  const [busy, setBusy] = useState(false);

  // Reset transient UI when the active request changes.
  useEffect(() => {
    setShowAlways(false);
    setScope("project");
    setBusy(false);
  }, [req?.id]);

  useEffect(() => {
    if (!req) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) decide(req.id, "ask", "user dismissed");
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [req, busy, decide]);

  const suggestions: RuleSuggestion[] = useMemo(
    () => (req ? suggestRules(req, scope) : []),
    [req, scope],
  );

  if (!req) return null;
  const { primary, secondary } = describe(req);

  return (
    <div className="modal-backdrop">
      <div className="modal approval-modal" onClick={(e) => e.stopPropagation()}>
        <div className="approval-head">
          <span className={`approval-tool tool-${req.tool.toLowerCase()}`}>{req.tool}</span>
          <span className="approval-cwd" title={req.cwd}>{shortenPath(req.cwd)}</span>
        </div>

        <pre className="approval-primary">{primary}</pre>
        {secondary && <div className="approval-secondary">{secondary}</div>}

        {!showAlways ? (
          <div className="approval-actions">
            <button
              className="btn-deny"
              disabled={busy}
              onClick={async () => { setBusy(true); await decide(req.id, "deny", "user denied"); }}
            >
              Deny
            </button>
            <button
              className="btn-secondary"
              disabled={busy}
              onClick={() => setShowAlways(true)}
            >
              Always…
            </button>
            <button
              className="btn-approve"
              disabled={busy}
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

  // Switch suggestion if scope change pushes it into a new risk class.
  useEffect(() => { setConfirmText(""); }, [selectedIdx, scope, denyMode]);

  if (!selected) return <div className="placeholder">No suggestions.</div>;

  // Recompute effect-specific classification (deny mode flips to deny).
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
        <button className="btn-secondary" onClick={onCancel}>Back</button>
        <button
          className="btn-approve"
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
