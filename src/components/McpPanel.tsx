import { useEffect, useState } from "react";

import { useMcpStore } from "../stores/mcpStore";
import { PanelError } from "./PanelError";
import type { McpServer } from "../types/domain";

type Transport = "stdio" | "http" | "sse";
type Scope = "local" | "user" | "project";

export function McpPanel() {
  const servers = useMcpStore((s) => s.servers);
  const loading = useMcpStore((s) => s.loading);
  const error = useMcpStore((s) => s.error);
  const removing = useMcpStore((s) => s.removing);
  const refresh = useMcpStore((s) => s.refresh);
  const remove = useMcpStore((s) => s.remove);

  const [adding, setAdding] = useState(false);

  useEffect(() => { void refresh(); }, [refresh]);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">mcp servers</span>
        <span className="spacer" />
        <button
          className="workbench-action"
          onClick={() => setAdding((v) => !v)}
          title="Add MCP server"
        >{adding ? "×" : "+"}</button>
        <button
          className="workbench-action"
          onClick={() => void refresh()}
          disabled={loading}
          title="Refresh (health-checks each server)"
        >{loading ? "…" : "↻"}</button>
      </div>

      <div className="workbench-body">
        {adding && <AddForm onDone={() => setAdding(false)} />}

        {error && <PanelError message={error} onRetry={refresh} />}

        <section className="wb-section">
          <p className="wb-hint wb-trust">
            An MCP server can supply tools and data to the agent — only add ones
            you trust.
          </p>
          <div className="wb-section-title">
            configured
            {servers.length > 0 && <span className="wb-count">{servers.length}</span>}
            {loading && <span className="plugins-loading"> · checking…</span>}
          </div>

          {servers.length === 0 && !loading ? (
            <p className="wb-hint">
              No MCP servers configured. Click <strong>+</strong> to add one, or
              run <code>claude mcp add</code> in a terminal.
            </p>
          ) : (
            <ul className="mcp-list">
              {servers.map((s) => (
                <McpRow
                  key={`${s.scope ?? "?"}:${s.name}`}
                  server={s}
                  busy={!!removing[s.name]}
                  onRemove={() => {
                    if (confirm(`Remove MCP server "${s.name}" from ${s.scope ?? "its"} scope?`)) {
                      void remove(s.name, s.scope ?? "local");
                    }
                  }}
                />
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

function McpRow({ server, busy, onRemove }: {
  server: McpServer;
  busy: boolean;
  onRemove: () => void;
}) {
  const [open, setOpen] = useState(false);
  const target = server.transport === "stdio"
    ? [server.command, server.args].filter(Boolean).join(" ")
    : (server.url ?? "");

  return (
    <li className="mcp-row">
      <div className="mcp-row-head" onClick={() => setOpen((v) => !v)}>
        <span className="caret">{open ? "▾" : "▸"}</span>
        <span className={`mcp-status mcp-status-${server.status}`} title={server.status} />
        <div className="mcp-row-text">
          <div className="mcp-name">{server.name}</div>
          <div className="mcp-target">{target}</div>
        </div>
        <span className="mcp-badge">{server.scope ?? "?"}</span>
      </div>

      {open && (
        <div className="mcp-row-body">
          <dl className="mcp-detail">
            <dt>status</dt><dd>{server.status}</dd>
            <dt>transport</dt><dd>{server.transport ?? "—"}</dd>
            <dt>scope</dt><dd>{server.scope ?? "—"}</dd>
            {server.transport === "stdio" ? (
              <>
                <dt>command</dt><dd>{server.command ?? "—"}</dd>
                {server.args && (<><dt>args</dt><dd>{server.args}</dd></>)}
              </>
            ) : (
              <><dt>url</dt><dd className="mcp-mono">{server.url ?? "—"}</dd></>
            )}
            {server.env.length > 0 && (
              <><dt>env</dt><dd className="mcp-mono">{server.env.join("\n")}</dd></>
            )}
          </dl>
          <div className="mcp-row-actions">
            <button className="wb-link mcp-remove" onClick={onRemove} disabled={busy}>
              {busy ? "removing…" : "remove"}
            </button>
          </div>
        </div>
      )}
    </li>
  );
}

function AddForm({ onDone }: { onDone: () => void }) {
  const add = useMcpStore((s) => s.add);
  const adding = useMcpStore((s) => s.adding);
  const addError = useMcpStore((s) => s.addError);
  const clearAddError = useMcpStore((s) => s.clearAddError);

  const [name, setName] = useState("");
  const [transport, setTransport] = useState<Transport>("stdio");
  const [scope, setScope] = useState<Scope>("local");
  const [commandOrUrl, setCommandOrUrl] = useState("");
  const [envText, setEnvText] = useState("");
  const [headersText, setHeadersText] = useState("");

  const stdio = transport === "stdio";
  const canSubmit = name.trim() !== "" && commandOrUrl.trim() !== "" && !adding;

  const submit = async () => {
    if (!canSubmit) return;
    const env = envText.split("\n").map((l) => l.trim()).filter((l) => l.includes("="));
    const headers = headersText.split("\n").map((l) => l.trim()).filter((l) => l.includes(":"));
    const ok = await add({
      name: name.trim(),
      transport,
      scope,
      commandOrUrl: commandOrUrl.trim(),
      env: stdio ? env : [],
      headers: stdio ? [] : headers,
    });
    if (ok) onDone();
  };

  return (
    <section className="wb-section mcp-add">
      <div className="wb-section-title">add server</div>

      <div className="mcp-field-row">
        <label className="mcp-field">
          <span>name</span>
          <input
            className="wb-search-input"
            value={name}
            placeholder="e.g. github"
            onChange={(e) => { setName(e.target.value); clearAddError(); }}
            autoFocus
          />
        </label>
        <label className="mcp-field mcp-field-sm">
          <span>transport</span>
          <select value={transport} onChange={(e) => setTransport(e.target.value as Transport)}>
            <option value="stdio">stdio</option>
            <option value="http">http</option>
            <option value="sse">sse</option>
          </select>
        </label>
        <label className="mcp-field mcp-field-sm">
          <span>scope</span>
          <select value={scope} onChange={(e) => setScope(e.target.value as Scope)}>
            <option value="local">local</option>
            <option value="user">user</option>
            <option value="project">project</option>
          </select>
        </label>
      </div>

      <label className="mcp-field">
        <span>{stdio ? "command (with args)" : "url"}</span>
        <input
          className="wb-search-input"
          value={commandOrUrl}
          placeholder={stdio ? "npx -y @modelcontextprotocol/server-github" : "https://mcp.example.dev/mcp"}
          onChange={(e) => { setCommandOrUrl(e.target.value); clearAddError(); }}
        />
      </label>

      {stdio ? (
        <label className="mcp-field">
          <span>env (KEY=VALUE per line, optional)</span>
          <textarea
            className="mcp-textarea"
            rows={2}
            value={envText}
            placeholder="GITHUB_TOKEN=ghp_…"
            onChange={(e) => setEnvText(e.target.value)}
          />
        </label>
      ) : (
        <label className="mcp-field">
          <span>headers (Name: Value per line, optional)</span>
          <textarea
            className="mcp-textarea"
            rows={2}
            value={headersText}
            placeholder="Authorization: Bearer …"
            onChange={(e) => setHeadersText(e.target.value)}
          />
        </label>
      )}

      {addError && <div className="plugin-install-error">{addError}</div>}

      <div className="mcp-add-actions">
        <button className="wb-link" onClick={onDone}>cancel</button>
        <button className="wb-cta wb-cta-sm" onClick={() => void submit()} disabled={!canSubmit}>
          {adding ? "adding…" : "Add server"}
        </button>
      </div>
    </section>
  );
}
