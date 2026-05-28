import { useEffect, useMemo, useState } from "react";

import { usePluginsStore } from "../stores/pluginsStore";

export function PluginsPanel() {
  const installed = usePluginsStore((s) => s.installed);
  const marketplace = usePluginsStore((s) => s.marketplace);
  const query = usePluginsStore((s) => s.query);
  const installedLoading = usePluginsStore((s) => s.installedLoading);
  const marketplaceLoading = usePluginsStore((s) => s.marketplaceLoading);
  const marketplaceSource = usePluginsStore((s) => s.marketplaceSource);
  const marketplaceIsFallback = usePluginsStore((s) => s.marketplaceIsFallback);
  const marketplaceFetchedAtMs = usePluginsStore((s) => s.marketplaceFetchedAtMs);
  const error = usePluginsStore((s) => s.error);
  const setQuery = usePluginsStore((s) => s.setQuery);
  const refreshInstalled = usePluginsStore((s) => s.refreshInstalled);
  const refreshMarketplace = usePluginsStore((s) => s.refreshMarketplace);
  const installViaTerminal = usePluginsStore((s) => s.installViaTerminal);

  const [flashMsg, setFlashMsg] = useState<string | null>(null);

  useEffect(() => {
    void refreshInstalled();
    void refreshMarketplace(false);
  }, [refreshInstalled, refreshMarketplace]);

  const installedSlugs = useMemo(() => new Set(installed.map((p) => p.slug)), [installed]);
  const q = query.trim().toLowerCase();

  const filteredInstalled = useMemo(() => installed.filter((p) =>
    !q || p.name.toLowerCase().includes(q) || (p.description ?? "").toLowerCase().includes(q),
  ), [installed, q]);

  const filteredMarket = useMemo(() => marketplace.filter((p) => {
    if (installedSlugs.has(p.slug)) return false;
    if (!q) return true;
    return p.name.toLowerCase().includes(q)
      || p.description.toLowerCase().includes(q)
      || p.tags.some((t) => t.toLowerCase().includes(q));
  }), [marketplace, installedSlugs, q]);

  const onInstall = (slug: string) => {
    const err = installViaTerminal(slug);
    if (err === "no-active-session") {
      setFlashMsg("Abrí una sesión de terminal antes de instalar.");
      setTimeout(() => setFlashMsg(null), 3000);
    } else {
      setFlashMsg(`Enviado al terminal: /plugin install ${slug}`);
      setTimeout(() => setFlashMsg(null), 2500);
    }
  };

  return (
    <div className="plugins-panel">
      <div className="plugins-header">
        <input
          className="plugins-search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Buscar plugins (nombre, descripción, tag)…"
          spellCheck={false}
        />
        <button
          className="plugins-refresh"
          onClick={() => { void refreshInstalled(); void refreshMarketplace(true); }}
          title="Force refresh"
        >↻</button>
      </div>

      {flashMsg && <div className="plugins-flash">{flashMsg}</div>}
      {error && <div className="plugins-error">{error}</div>}

      <Section
        title={`Installed${installed.length ? ` (${installed.length})` : ""}`}
        loading={installedLoading}
        empty={installed.length === 0 ? "Sin plugins instalados en ~/.claude/plugins/." : undefined}
      >
        {filteredInstalled.map((p) => (
          <div key={p.slug} className="plugin-row installed">
            <div className="plugin-row-main">
              <div className="plugin-name">
                {p.name}
                {p.version && <span className="plugin-version">v{p.version}</span>}
              </div>
              {p.description && <div className="plugin-desc">{p.description}</div>}
              <div className="plugin-meta">{p.path}</div>
            </div>
            <span className="plugin-badge installed">installed</span>
          </div>
        ))}
        {installed.length > 0 && filteredInstalled.length === 0 && (
          <div className="plugins-empty-sub">Sin coincidencias en instalados.</div>
        )}
      </Section>

      <Section
        title={`Recommended${filteredMarket.length ? ` (${filteredMarket.length})` : ""}`}
        loading={marketplaceLoading}
        empty={marketplace.length === 0 ? "Marketplace vacío." : undefined}
      >
        {filteredMarket.map((p) => (
          <div key={p.slug} className="plugin-row">
            <div className="plugin-row-main">
              <div className="plugin-name">{p.name}</div>
              {p.description && <div className="plugin-desc">{p.description}</div>}
              <div className="plugin-meta">
                {p.author && <span>by {p.author} · </span>}
                {p.tags.length > 0 && <span>{p.tags.map((t) => `#${t}`).join(" ")}</span>}
              </div>
            </div>
            <div className="plugin-actions">
              {p.repoUrl && (
                <a className="plugin-link" href={p.repoUrl} target="_blank" rel="noreferrer" title="Open repository">↗</a>
              )}
              <button
                className="plugin-install"
                onClick={() => onInstall(p.slug)}
                title={`Run /plugin install ${p.slug} in the active terminal`}
              >Install</button>
            </div>
          </div>
        ))}
        {marketplace.length > 0 && filteredMarket.length === 0 && (
          <div className="plugins-empty-sub">Sin coincidencias en recomendados.</div>
        )}
      </Section>

      <div className="plugins-footer">
        <span>
          {marketplaceSource ? <>Marketplace: <code>{marketplaceSource}</code></> : "Marketplace: —"}
          {marketplaceIsFallback && <span className="plugins-warn"> · fallback</span>}
          {marketplaceFetchedAtMs && (
            <span className="plugins-muted"> · {new Date(marketplaceFetchedAtMs).toLocaleString()}</span>
          )}
        </span>
      </div>
    </div>
  );
}

function Section({ title, loading, empty, children }: {
  title: string;
  loading: boolean;
  empty?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="plugins-section">
      <div className="plugins-section-title">
        {title}
        {loading && <span className="plugins-loading"> · loading…</span>}
      </div>
      <div className="plugins-section-body">
        {empty && !loading ? <div className="plugins-empty">{empty}</div> : children}
      </div>
    </section>
  );
}
