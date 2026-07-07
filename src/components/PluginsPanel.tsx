import { useEffect, useMemo } from "react";

import { usePluginsStore } from "../stores/pluginsStore";

export function PluginsPanel() {
  const installed = usePluginsStore((s) => s.installed);
  const available = usePluginsStore((s) => s.available);
  const marketplaces = usePluginsStore((s) => s.marketplaces);
  const query = usePluginsStore((s) => s.query);
  const installedLoading = usePluginsStore((s) => s.installedLoading);
  const availableLoading = usePluginsStore((s) => s.availableLoading);
  const installing = usePluginsStore((s) => s.installing);
  const installErrors = usePluginsStore((s) => s.installErrors);
  const updating = usePluginsStore((s) => s.updating);
  const updateErrors = usePluginsStore((s) => s.updateErrors);
  const updatingAll = usePluginsStore((s) => s.updatingAll);
  const error = usePluginsStore((s) => s.error);
  const setQuery = usePluginsStore((s) => s.setQuery);
  const refreshInstalled = usePluginsStore((s) => s.refreshInstalled);
  const refreshAvailable = usePluginsStore((s) => s.refreshAvailable);
  const install = usePluginsStore((s) => s.install);
  const update = usePluginsStore((s) => s.update);
  const updateAll = usePluginsStore((s) => s.updateAll);

  useEffect(() => {
    void refreshInstalled();
    void refreshAvailable();
  }, [refreshInstalled, refreshAvailable]);

  const installedIds = useMemo(() => new Set(installed.map((p) => p.id)), [installed]);
  const q = query.trim().toLowerCase();

  const filteredInstalled = useMemo(() => installed.filter((p) =>
    !q || p.name.toLowerCase().includes(q) || (p.marketplace ?? "").toLowerCase().includes(q),
  ), [installed, q]);

  const filteredAvailable = useMemo(() => available.filter((p) => {
    if (installedIds.has(p.installId)) return false;
    if (!q) return true;
    return p.name.toLowerCase().includes(q)
      || p.description.toLowerCase().includes(q)
      || (p.category ?? "").toLowerCase().includes(q)
      || p.marketplace.toLowerCase().includes(q);
  }), [available, installedIds, q]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">plugins</span>
        <input
          className="plugins-search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search plugins (name, description, category)…"
          spellCheck={false}
        />
        <button
          className="workbench-action"
          onClick={() => { void refreshInstalled(); void refreshAvailable(); }}
          title="Refresh"
        >↻</button>
      </div>

      <div className="workbench-body">
      {error && <div className="plugins-error">{error}</div>}

      <Section
        title={`Installed${installed.length ? ` (${installed.length})` : ""}`}
        loading={installedLoading}
        empty={installed.length === 0 ? "No plugins installed yet." : undefined}
        action={
          installed.length > 0 ? (
            <button
              className="plugin-update"
              onClick={() => void updateAll()}
              disabled={updatingAll}
              title="Refresh marketplaces and update every installed plugin"
            >
              {updatingAll ? <span className="plugin-spinner" aria-label="updating" /> : "Update all"}
            </button>
          ) : undefined
        }
      >
        {filteredInstalled.map((p) => {
          const busy = !!updating[p.id];
          const err = updateErrors[p.id];
          return (
            <div key={p.id} className="plugin-row installed">
              <div className="plugin-row-main">
                <div className="plugin-name">
                  {p.name}
                  {p.version && <span className="plugin-version">v{p.version}</span>}
                  {!p.enabled && <span className="plugin-version">disabled</span>}
                </div>
                <div className="plugin-meta">
                  {p.marketplace && <span>{p.marketplace}</span>}
                  {p.scope && <span> · {p.scope}</span>}
                </div>
                {err && <div className="plugin-install-error">{err}</div>}
              </div>
              <button
                className="plugin-update"
                onClick={() => void update(p.id)}
                disabled={busy || updatingAll}
                title={`Update ${p.id} to the latest version (restart sessions to apply)`}
              >
                {busy ? <span className="plugin-spinner" aria-label="updating" /> : "Update"}
              </button>
              <span className="plugin-badge installed">{p.enabled ? "installed" : "off"}</span>
            </div>
          );
        })}
        {installed.length > 0 && filteredInstalled.length === 0 && (
          <div className="plugins-empty-sub">No matches in installed.</div>
        )}
      </Section>

      <Section
        title={`Available${filteredAvailable.length ? ` (${filteredAvailable.length})` : ""}`}
        loading={availableLoading}
        empty={
          available.length === 0
            ? (marketplaces.length === 0
                ? "No marketplaces configured. Add one with: claude plugin marketplace add <repo>"
                : "No plugins found in the configured marketplaces.")
            : undefined
        }
      >
        {filteredAvailable.map((p) => {
          const busy = !!installing[p.installId];
          const err = installErrors[p.installId];
          return (
            <div key={p.installId} className="plugin-row">
              <div className="plugin-row-main">
                <div className="plugin-name">{p.name}</div>
                {p.description && <div className="plugin-desc">{p.description}</div>}
                <div className="plugin-meta">
                  <span>{p.marketplace}</span>
                  {p.category && <span> · {p.category}</span>}
                  {p.author && <span> · by {p.author}</span>}
                </div>
                {err && <div className="plugin-install-error">{err}</div>}
              </div>
              <div className="plugin-actions">
                {p.homepage && (
                  <a className="plugin-link" href={p.homepage} target="_blank" rel="noreferrer" title="Open homepage">↗</a>
                )}
                <button
                  className="plugin-install"
                  onClick={() => void install(p.installId)}
                  disabled={busy}
                  title={`Install ${p.installId} (user scope)`}
                >
                  {busy ? <span className="plugin-spinner" aria-label="installing" /> : "Install"}
                </button>
              </div>
            </div>
          );
        })}
        {available.length > 0 && filteredAvailable.length === 0 && (
          <div className="plugins-empty-sub">No matches in available.</div>
        )}
      </Section>

      <div className="plugins-footer">
        <span>
          {marketplaces.length > 0
            ? <>Marketplaces: <code>{marketplaces.join(", ")}</code></>
            : "No marketplaces configured"}
        </span>
      </div>
      </div>
    </div>
  );
}

function Section({ title, loading, empty, action, children }: {
  title: string;
  loading: boolean;
  empty?: string;
  /// Optional right-aligned action in the section title row (e.g. Update all).
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="plugins-section">
      <div className="plugins-section-title">
        {title}
        {loading && <span className="plugins-loading"> · loading…</span>}
        {action && <span className="plugins-section-action">{action}</span>}
      </div>
      <div className="plugins-section-body">
        {empty && !loading ? <div className="plugins-empty">{empty}</div> : children}
      </div>
    </section>
  );
}
