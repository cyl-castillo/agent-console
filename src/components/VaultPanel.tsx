import { useEffect, useMemo, useState } from "react";

import { useVaultStore } from "../stores/vaultStore";
import { useSessionStore } from "../stores/sessionStore";
import { PanelError } from "./PanelError";
import type { VaultEntryView } from "../types/domain";

type Scope = "project" | "global";

export function VaultPanel() {
  const entries = useVaultStore((s) => s.entries);
  const loading = useVaultStore((s) => s.loading);
  const error = useVaultStore((s) => s.error);
  const refresh = useVaultStore((s) => s.refresh);
  const project = useSessionStore((s) => s.project);

  const [adding, setAdding] = useState(false);
  const [editingKey, setEditingKey] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  useEffect(() => { refresh(); }, [refresh]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((e) =>
      e.key.toLowerCase().includes(q) || e.description.toLowerCase().includes(q),
    );
  }, [entries, query]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">vault</span>
        <span className="spacer" />
        <button className="workbench-action" onClick={refresh} disabled={loading} title="Refresh">↻</button>
        <button
          className="workbench-action"
          onClick={() => { setEditingKey(null); setAdding(true); }}
          title="Add entry"
        >＋</button>
      </div>

      <div className="workbench-body">
        <section className="wb-section">
          <p className="wb-hint">
            Entries are injected as environment variables into every terminal
            this app spawns. Secrets live in your OS keychain; non-secrets in
            <code> .claude/vault.json</code>. A companion <code>VAULT.md</code>
            lists key names + descriptions so Claude knows what's available.
          </p>
          <p className="wb-hint wb-trust">
            Values stay on your machine and are never sent to the model — the
            agent sees only the key names.
          </p>
        </section>

        {error && (
          <section className="wb-section">
            <PanelError message={error} onRetry={refresh} />
          </section>
        )}

        {adding && (
          <section className="wb-section">
            <VaultEntryForm
              defaultScope={project ? "project" : "global"}
              projectAvailable={!!project}
              onCancel={() => setAdding(false)}
              onSaved={() => setAdding(false)}
            />
          </section>
        )}

        <section className="wb-section">
          <div className="wb-section-title">
            entries
            {entries.length > 0 && <span className="wb-count">{entries.length}</span>}
          </div>

          {entries.length > 0 && (
            <div className="wb-search">
              <input
                className="wb-search-input"
                placeholder="Search entries…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
              {query && (
                <button className="wb-search-clear" onClick={() => setQuery("")} title="Clear">×</button>
              )}
            </div>
          )}

          {entries.length === 0 && loading ? (
            <p className="wb-hint">Loading…</p>
          ) : entries.length === 0 ? (
            <p className="wb-hint">
              No entries yet. Click <strong>＋</strong> to add a password, token,
              URL, or any default the agent should reuse.
            </p>
          ) : filtered.length === 0 ? (
            <p className="wb-hint">No matches.</p>
          ) : (
            <ul className="vault-list">
              {filtered.map((e) => (
                <VaultRow
                  key={`${e.scope}:${e.key}`}
                  entry={e}
                  editing={editingKey === `${e.scope}:${e.key}`}
                  onEdit={() => { setAdding(false); setEditingKey(`${e.scope}:${e.key}`); }}
                  onCancelEdit={() => setEditingKey(null)}
                  onSaved={() => setEditingKey(null)}
                  projectAvailable={!!project}
                />
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

function VaultRow({
  entry, editing, onEdit, onCancelEdit, onSaved, projectAvailable,
}: {
  entry: VaultEntryView;
  editing: boolean;
  onEdit: () => void;
  onCancelEdit: () => void;
  onSaved: () => void;
  projectAvailable: boolean;
}) {
  const remove = useVaultStore((s) => s.remove);
  const reveal = useVaultStore((s) => s.reveal);
  const [shown, setShown] = useState<string | null>(null);
  const [revealing, setRevealing] = useState(false);
  const [copied, setCopied] = useState(false);

  if (editing) {
    return (
      <li className="vault-edit-row">
        <VaultEntryForm
          existing={entry}
          defaultScope={entry.scope}
          projectAvailable={projectAvailable}
          onCancel={onCancelEdit}
          onSaved={onSaved}
        />
      </li>
    );
  }

  const onReveal = async () => {
    if (shown !== null) { setShown(null); return; }
    setRevealing(true);
    try {
      const v = await reveal(entry.scope, entry.key);
      setShown(v);
    } catch (e) {
      alert(`Could not reveal: ${e}`);
    } finally {
      setRevealing(false);
    }
  };

  const onCopy = async () => {
    try {
      const v = shown ?? (await reveal(entry.scope, entry.key));
      await navigator.clipboard?.writeText(v);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch (e) {
      alert(`Could not copy: ${e}`);
    }
  };

  const onDelete = () => {
    if (confirm(`Delete vault entry "${entry.key}"? This cannot be undone.`)) {
      remove(entry.scope, entry.key);
    }
  };

  return (
    <li className="vault-row" title={entry.description}>
      <div className="vault-row-head">
        <span className={`vault-kind kind-${entry.secret ? "secret" : "config"}`}>
          {entry.secret ? "S" : "C"}
        </span>
        <div className="vault-text">
          <div className="vault-key">${entry.key}
            <span className="vault-scope">{entry.scope}</span>
          </div>
          {entry.description && <div className="vault-desc">{entry.description}</div>}
        </div>
        <div className="vault-actions">
          <button onClick={onReveal} disabled={revealing} title={shown !== null ? "Hide" : "Reveal"}>
            {revealing ? "…" : shown !== null ? "🙈" : "👁"}
          </button>
          <button onClick={onCopy} title="Copy value">{copied ? "✓" : "⧉"}</button>
          <button onClick={onEdit} title="Edit">✎</button>
          <button onClick={onDelete} title="Delete">×</button>
        </div>
      </div>
      {shown !== null && (
        <pre className="vault-value">{shown || "(empty)"}</pre>
      )}
    </li>
  );
}

function VaultEntryForm({
  existing, defaultScope, projectAvailable, onCancel, onSaved,
}: {
  existing?: VaultEntryView;
  defaultScope: Scope;
  projectAvailable: boolean;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const upsert = useVaultStore((s) => s.upsert);
  const [key, setKey] = useState(existing?.key ?? "");
  const [scope, setScope] = useState<Scope>(existing?.scope ?? defaultScope);
  const [description, setDescription] = useState(existing?.description ?? "");
  const [secret, setSecret] = useState(existing?.secret ?? true);
  const [value, setValue] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const editing = !!existing;
  const valueRequired = !editing; // on edit, leaving value empty keeps existing

  const onSave = async () => {
    setErr(null);
    setSubmitting(true);
    try {
      await upsert({
        scope,
        key: key.trim(),
        description: description.trim(),
        secret,
        value: value.length > 0 ? value : null,
      });
      onSaved();
    } catch (e) {
      setErr(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="vault-form">
      <div className="vault-form-row">
        <label>key</label>
        <input
          value={key}
          onChange={(e) => setKey(e.target.value.toUpperCase())}
          placeholder="DATABASE_URL"
          disabled={editing || submitting}
        />
      </div>
      <div className="vault-form-row">
        <label>description</label>
        <input
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Staging Postgres URL for migrations"
          disabled={submitting}
        />
      </div>
      <div className="vault-form-row">
        <label>scope</label>
        <select
          value={scope}
          onChange={(e) => setScope(e.target.value as Scope)}
          disabled={editing || submitting}
        >
          <option value="project" disabled={!projectAvailable}>project (this repo)</option>
          <option value="global">global (all projects)</option>
        </select>
      </div>
      <div className="vault-form-row">
        <label>kind</label>
        <div className="vault-form-radio">
          <label><input type="radio" checked={secret} onChange={() => setSecret(true)} /> secret (keychain)</label>
          <label><input type="radio" checked={!secret} onChange={() => setSecret(false)} /> config (plaintext)</label>
        </div>
      </div>
      <div className="vault-form-row">
        <label>value</label>
        <input
          type={secret ? "password" : "text"}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={editing ? "(leave empty to keep current)" : ""}
          disabled={submitting}
        />
      </div>
      {err && <div className="vault-form-error">{err}</div>}
      <div className="vault-form-actions">
        <button className="wb-link" onClick={onCancel} disabled={submitting}>cancel</button>
        <button
          className="wb-cta wb-cta-sm"
          onClick={onSave}
          disabled={submitting || !key.trim() || (valueRequired && !value && secret)}
        >
          {submitting ? "saving…" : editing ? "Save" : "Add"}
        </button>
      </div>
    </div>
  );
}
