import { useEffect, useMemo, useState } from "react";

import { usePermissionsStore } from "../stores/permissionsStore";
import { classify, parseRaw, buildRaw, isHardDenyAllow } from "../permissions/rules";
import type { StoredRule } from "../types/domain";

type Scope = "project" | "global";
type Effect = "allow" | "deny" | "ask";

const KNOWN_TOOLS = [
  "Bash", "Edit", "Write", "MultiEdit", "Read", "NotebookEdit",
  "WebFetch", "WebSearch", "Glob", "Grep",
];

interface FilterState {
  query: string;
  showExternal: boolean;
  effects: Record<Effect, boolean>;
}

export function PermissionsPanel() {
  const snapshot = usePermissionsStore((s) => s.snapshot);
  const loading = usePermissionsStore((s) => s.loading);
  const lastOp = usePermissionsStore((s) => s.lastOp);
  const error = usePermissionsStore((s) => s.error);
  const refresh = usePermissionsStore((s) => s.refresh);
  const remove = usePermissionsStore((s) => s.remove);
  const undo = usePermissionsStore((s) => s.undo);
  const move = usePermissionsStore((s) => s.move);
  const clearError = usePermissionsStore((s) => s.clearError);

  useEffect(() => { refresh(); }, [refresh]);

  const [formOpen, setFormOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<StoredRule | null>(null);
  const [filter, setFilter] = useState<FilterState>({
    query: "",
    showExternal: true,
    effects: { allow: true, deny: true, ask: true },
  });
  const [onlyFlagged, setOnlyFlagged] = useState(false);

  const allRules = snapshot?.rules ?? [];

  const flaggedCount = useMemo(
    () => allRules.filter((r) => {
      if (r.effect !== "allow") return false;
      const p = parseRaw(r.raw);
      if (!p) return false;
      const risk = classify({ scope: r.scope, effect: r.effect, tool: p.tool, pattern: p.pattern, raw: r.raw }).risk;
      return risk === "broad" || risk === "dangerous";
    }).length,
    [allRules],
  );

  const filteredRules = useMemo(() => allRules.filter((r) => {
    if (!filter.effects[r.effect]) return false;
    if (!filter.showExternal && r.source === "external") return false;
    if (filter.query && !r.raw.toLowerCase().includes(filter.query.toLowerCase())) return false;
    if (onlyFlagged) {
      const p = parseRaw(r.raw);
      if (!p) return false;
      const risk = classify({ scope: r.scope, effect: r.effect, tool: p.tool, pattern: p.pattern, raw: r.raw }).risk;
      if (r.effect !== "allow" || (risk !== "broad" && risk !== "dangerous")) return false;
    }
    return true;
  }), [allRules, filter, onlyFlagged]);

  const grouped = useMemo(() => {
    const project: StoredRule[] = [];
    const global: StoredRule[] = [];
    for (const r of filteredRules) (r.scope === "project" ? project : global).push(r);
    return { project, global };
  }, [filteredRules]);

  return (
    <div className="permissions-panel">
      <div className="panel-header">
        <span>Permissions</span>
        <div className="perm-header-actions">
          {lastOp && (
            <button className="link-btn" onClick={undo} title="Undo last change">↶ undo</button>
          )}
          <button className="link-btn" onClick={() => refresh()} disabled={loading}>refresh</button>
          <button
            className="link-btn primary"
            onClick={() => { setEditTarget(null); setFormOpen(true); }}
          >+ add</button>
        </div>
      </div>

      {error && (
        <div className="modal-error" style={{ margin: "0 10px" }}>
          {error}
          <button className="link-btn" style={{ marginLeft: 8 }} onClick={clearError}>dismiss</button>
        </div>
      )}

      {flaggedCount > 0 && (
        <div
          className="lint-banner"
          onClick={() => setOnlyFlagged((v) => !v)}
          title="Click to filter"
        >
          ⚠ {flaggedCount} broad/dangerous allow rule{flaggedCount === 1 ? "" : "s"} —
          {" "}{onlyFlagged ? "showing only flagged" : "review them"}
        </div>
      )}

      <FilterBar filter={filter} setFilter={setFilter} />

      {(formOpen || editTarget) && (
        <RuleForm
          edit={editTarget}
          defaultScope={editTarget?.scope ?? (snapshot?.projectSettingsPath ? "project" : "global")}
          hasProject={!!snapshot?.projectSettingsPath}
          onClose={() => { setFormOpen(false); setEditTarget(null); }}
        />
      )}

      {!snapshot ? (
        <div className="placeholder">{loading ? "Loading…" : "No data."}</div>
      ) : (
        <>
          <RuleGroup
            title="Project"
            path={snapshot.projectSettingsPath ?? "(no project open)"}
            rules={grouped.project}
            disabled={!snapshot.projectSettingsPath}
            onRemove={(r) => remove(r.scope, r.effect, r.raw)}
            onEdit={(r) => setEditTarget(r)}
            onMove={(r) => move(r, "global")}
            moveLabel="→ global"
          />
          <RuleGroup
            title="Global"
            path={snapshot.globalSettingsPath}
            rules={grouped.global}
            disabled={false}
            onRemove={(r) => remove(r.scope, r.effect, r.raw)}
            onEdit={(r) => setEditTarget(r)}
            onMove={snapshot.projectSettingsPath ? (r) => move(r, "project") : undefined}
            moveLabel="→ project"
          />
        </>
      )}
    </div>
  );
}

// --- FilterBar ---------------------------------------------------------------

function FilterBar({ filter, setFilter }: { filter: FilterState; setFilter: (f: FilterState) => void }) {
  return (
    <div className="perm-filter">
      <input
        type="text"
        placeholder="filter rules…"
        value={filter.query}
        onChange={(e) => setFilter({ ...filter, query: e.target.value })}
      />
      <div className="perm-filter-toggles">
        {(["allow", "deny", "ask"] as const).map((e) => (
          <label key={e} className={`chip eff-${e} ${filter.effects[e] ? "on" : ""}`}>
            <input
              type="checkbox"
              checked={filter.effects[e]}
              onChange={(ev) => setFilter({ ...filter, effects: { ...filter.effects, [e]: ev.target.checked } })}
            />
            {e}
          </label>
        ))}
        <label className={`chip ${filter.showExternal ? "on" : ""}`}>
          <input
            type="checkbox"
            checked={filter.showExternal}
            onChange={(e) => setFilter({ ...filter, showExternal: e.target.checked })}
          />
          ext
        </label>
      </div>
    </div>
  );
}

// --- RuleForm (add + edit) ---------------------------------------------------

interface RuleFormProps {
  edit: StoredRule | null;
  defaultScope: Scope;
  hasProject: boolean;
  onClose: () => void;
}

function RuleForm({ edit, defaultScope, hasProject, onClose }: RuleFormProps) {
  const add = usePermissionsStore((s) => s.add);
  const remove = usePermissionsStore((s) => s.remove);

  const parsed = edit ? parseRaw(edit.raw) : null;
  const [scope, setScope] = useState<Scope>(edit?.scope ?? defaultScope);
  const [effect, setEffect] = useState<Effect>(edit?.effect ?? "allow");
  const [tool, setTool] = useState<string>(parsed?.tool ?? "Bash");
  const [pattern, setPattern] = useState<string>(parsed?.pattern ?? "");
  const [confirmText, setConfirmText] = useState("");
  const [busy, setBusy] = useState(false);

  const patternOrNull = pattern.trim() === "" ? null : pattern;
  const raw = buildRaw(tool, patternOrNull);
  const { risk, reason } = classify({ scope, effect, tool, pattern: patternOrNull, raw });
  const hd = effect === "allow" ? isHardDenyAllow(raw) : { hard: false } as const;
  const blocked = hd.hard;
  const strict = risk === "broad" || risk === "dangerous";
  const requiresTyping = !blocked && (
    risk === "dangerous" ||
    (effect === "allow" && scope === "global" && (tool === "Bash" || tool === "Write"))
  );

  const canSave = !blocked && (
    (requiresTyping && confirmText === raw) ||
    (!requiresTyping && strict && confirmText === "ok") ||
    (!requiresTyping && !strict)
  );

  const onSave = async () => {
    setBusy(true);
    if (edit) {
      // remove old + add new (atomic at the UI level only).
      await remove(edit.scope, edit.effect, edit.raw, false);
    }
    const r = await add(scope, effect, raw);
    setBusy(false);
    if (r || !edit) onClose();
  };

  return (
    <div className="perm-form">
      <div className="perm-form-head">{edit ? "Edit rule" : "New rule"}</div>

      <div className="perm-form-row">
        <span className="label">Scope</span>
        <div className="seg">
          <button
            className={scope === "project" ? "on" : ""}
            disabled={!hasProject}
            onClick={() => setScope("project")}
          >project</button>
          <button
            className={scope === "global" ? "on" : ""}
            onClick={() => setScope("global")}
          >global</button>
        </div>

        <span className="label" style={{ marginLeft: 10 }}>Effect</span>
        <div className="seg">
          {(["allow", "deny", "ask"] as const).map((e) => (
            <button key={e} className={effect === e ? "on" : ""} onClick={() => setEffect(e)}>{e}</button>
          ))}
        </div>
      </div>

      <div className="perm-form-row">
        <span className="label">Tool</span>
        <select value={tool} onChange={(e) => setTool(e.target.value)}>
          {KNOWN_TOOLS.map((t) => <option key={t} value={t}>{t}</option>)}
        </select>
        <span className="label" style={{ marginLeft: 10 }}>Pattern</span>
        <input
          value={pattern}
          onChange={(e) => setPattern(e.target.value)}
          placeholder={tool === "Bash" ? "npm test:* (empty = whole tool)" : "src/**"}
          style={{ flex: 1 }}
        />
      </div>

      <div className="approval-preview">
        <div className="preview-line">
          <span className="label">Rule</span>
          <code>{raw}</code>
          <span className={`risk risk-${risk}`}>{risk}</span>
        </div>
        {reason && <div className="preview-reason">{reason}</div>}
        <div className="preview-line">
          <span className="label">File</span>
          <code className="path">
            {scope === "project" ? ".claude/settings.json" : "~/.claude/settings.json"}
          </code>
        </div>
      </div>

      {blocked && (
        <div className="approval-blocked">
          <strong>Blocked</strong> — hard-deny list: {hd.reason}
        </div>
      )}

      {requiresTyping && !blocked && (
        <div className="approval-typegate">
          <label>Type the rule exactly: <code>{raw}</code></label>
          <input value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder={raw} />
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

      <div className="perm-form-actions">
        <button className="btn-secondary" onClick={onClose} disabled={busy}>Cancel</button>
        <button className="btn-approve" disabled={!canSave || busy} onClick={onSave}>
          {edit ? "Save changes" : "Add rule"}
        </button>
      </div>
    </div>
  );
}

// --- RuleGroup ---------------------------------------------------------------

interface GroupProps {
  title: string;
  path: string;
  rules: StoredRule[];
  disabled: boolean;
  onRemove: (r: StoredRule) => void;
  onEdit: (r: StoredRule) => void;
  onMove?: (r: StoredRule) => void;
  moveLabel: string;
}

function RuleGroup({ title, path, rules, disabled, onRemove, onEdit, onMove, moveLabel }: GroupProps) {
  return (
    <div className="rule-group">
      <div className="rule-group-head">
        <span className="rule-group-title">{title}</span>
        <code className="rule-group-path" title={path}>{path}</code>
      </div>
      {disabled ? (
        <div className="placeholder">—</div>
      ) : rules.length === 0 ? (
        <div className="placeholder">No rules.</div>
      ) : (
        <ul className="rule-list">
          {rules.map((r) => {
            const p = parseRaw(r.raw);
            const risk = p
              ? classify({ scope: r.scope, effect: r.effect, tool: p.tool, pattern: p.pattern, raw: r.raw }).risk
              : "safe";
            return (
              <li key={`${r.effect}::${r.raw}`} className="rule-item">
                <span className={`rule-effect eff-${r.effect}`}>{r.effect}</span>
                <code className="rule-raw">{r.raw}</code>
                {(risk === "broad" || risk === "dangerous") && r.effect === "allow" && (
                  <span className={`risk risk-${risk}`}>{risk}</span>
                )}
                {r.source === "external" && <span className="rule-src">ext</span>}
                <button className="rule-act" title="Edit" onClick={() => onEdit(r)}>✎</button>
                {onMove && <button className="rule-act" title="Move scope" onClick={() => onMove(r)}>{moveLabel}</button>}
                <button className="rule-rm" title="Remove rule" onClick={() => onRemove(r)}>×</button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
