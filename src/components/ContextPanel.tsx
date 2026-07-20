import { useEffect, useMemo, useState } from "react";

import { useContextStore } from "../stores/contextStore";
import { useSessionStore } from "../stores/sessionStore";
import { PanelError } from "./PanelError";
import type { ContextFileStat, MemoryEntry } from "../types/domain";
import { MarkdownText } from "./MarkdownText";

type Scope = "project" | "global";

export function ContextPanel() {
  const status = useContextStore((s) => s.status);
  const memories = useContextStore((s) => s.memories);
  const loading = useContextStore((s) => s.loading);
  const error = useContextStore((s) => s.error);
  const refresh = useContextStore((s) => s.refresh);
  const project = useSessionStore((s) => s.project);

  useEffect(() => {
    refresh();
  }, [refresh, project?.root]);

  const [projOpen, setProjOpen] = useState(true);
  const [globOpen, setGlobOpen] = useState(false);
  const [memOpen, setMemOpen] = useState(false);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">context</span>
        <span className="spacer" />
        <button className="workbench-action" onClick={refresh} disabled={loading} title="Refresh">
          ↻
        </button>
      </div>

      <div className="workbench-body">
        {error && (
          <section className="wb-section">
            <PanelError message={error} onRetry={refresh} />
          </section>
        )}

        <section className="wb-section">
          <button className="ctx-section-head scope-project" onClick={() => setProjOpen((v) => !v)}>
            <span className="caret">{projOpen ? "▾" : "▸"}</span>
            <span className="ctx-scope-badge scope-project">PROJECT</span>
            <span className="ctx-section-title">CLAUDE.md</span>
            {status?.projectClaudeMd && (
              <span className="ctx-section-meta">
                {status.projectClaudeMd.exists
                  ? `${formatSize(status.projectClaudeMd.sizeBytes)} · ${formatRelative(status.projectClaudeMd.modifiedMs)}`
                  : "missing"}
              </span>
            )}
          </button>
          {projOpen && project && status?.projectClaudeMd && (
            <ClaudeMdEditor scope="project" stat={status.projectClaudeMd} />
          )}
          {projOpen && !project && <p className="wb-hint">Open a project to view its CLAUDE.md.</p>}
        </section>

        <section className="wb-section">
          <button className="ctx-section-head scope-global" onClick={() => setGlobOpen((v) => !v)}>
            <span className="caret">{globOpen ? "▾" : "▸"}</span>
            <span className="ctx-scope-badge scope-global">GLOBAL</span>
            <span className="ctx-section-title">CLAUDE.md</span>
            {status?.globalClaudeMd && (
              <span className="ctx-section-meta">
                {status.globalClaudeMd.exists
                  ? `${formatSize(status.globalClaudeMd.sizeBytes)} · ${formatRelative(status.globalClaudeMd.modifiedMs)}`
                  : "missing"}
              </span>
            )}
          </button>
          {globOpen && status?.globalClaudeMd && (
            <ClaudeMdEditor scope="global" stat={status.globalClaudeMd} />
          )}
        </section>

        <section className="wb-section">
          <button className="ctx-section-head" onClick={() => setMemOpen((v) => !v)}>
            <span className="caret">{memOpen ? "▾" : "▸"}</span>
            <span className="ctx-section-title">Saved memories</span>
            {status?.memoryDir && (
              <span className="ctx-section-meta">
                {status.memoryDir.exists ? `${memories.length} files` : "no memory yet"}
              </span>
            )}
          </button>
          {memOpen && <MemoryList memories={memories} />}
        </section>
      </div>
    </div>
  );
}

function ClaudeMdEditor({ scope, stat }: { scope: Scope; stat: ContextFileStat }) {
  const readMd = useContextStore((s) => s.readMd);
  const writeMd = useContextStore((s) => s.writeMd);
  const openExternally = useContextStore((s) => s.openExternally);
  const generateStarter = useContextStore((s) => s.generateStarter);

  const [content, setContent] = useState<string>("");
  const [original, setOriginal] = useState<string>("");
  const [originalMtime, setOriginalMtime] = useState<number | null>(null);
  const [mode, setMode] = useState<"view" | "edit">("view");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [proposingStarter, setProposingStarter] = useState(false);
  const [proposed, setProposed] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  // Load whenever stat (path/mtime) changes.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setErr(null);
    readMd(scope)
      .then((c) => {
        if (cancelled) return;
        setContent(c);
        setOriginal(c);
        setOriginalMtime(stat.exists ? stat.modifiedMs : null);
        setMode("view");
      })
      .catch((e) => setErr(String(e)))
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [scope, stat.path, stat.modifiedMs, stat.exists, readMd]);

  const dirty = content !== original;

  const onSave = async () => {
    setErr(null);
    setSaving(true);
    try {
      await writeMd(scope, content, originalMtime);
      setOriginal(content);
      setMode("view");
    } catch (e) {
      const msg = String(e);
      if (msg.includes("context:conflict")) {
        if (
          confirm(
            "This file was modified externally since you opened it. Save anyway and overwrite?",
          )
        ) {
          try {
            await writeMd(scope, content, null);
            setOriginal(content);
            setMode("view");
          } catch (e2) {
            setErr(String(e2));
          }
        }
      } else {
        setErr(msg);
      }
    } finally {
      setSaving(false);
    }
  };

  const onProposeStarter = async () => {
    setProposingStarter(true);
    try {
      setProposed(await generateStarter());
    } catch (e) {
      setErr(String(e));
    } finally {
      setProposingStarter(false);
    }
  };

  const onAcceptStarter = async () => {
    if (proposed == null) return;
    setContent(proposed);
    setOriginal("");
    setMode("edit");
    setProposed(null);
  };

  if (loading) return <p className="wb-hint">Loading…</p>;

  if (!stat.exists && proposed == null) {
    return (
      <div className={`ctx-editor scope-${scope}`}>
        <p className="wb-hint">
          No <code>CLAUDE.md</code> at <code>{stat.path}</code> yet.
        </p>
        <div className="ctx-actions">
          {scope === "project" && (
            <button
              className="wb-cta wb-cta-sm"
              onClick={onProposeStarter}
              disabled={proposingStarter}
            >
              {proposingStarter ? "Generating…" : "Generate starter"}
            </button>
          )}
          <button
            className="wb-link"
            onClick={() => {
              setContent("");
              setOriginal("");
              setMode("edit");
            }}
          >
            Create empty
          </button>
        </div>
        {err && <p className="ctx-error">{err}</p>}
      </div>
    );
  }

  if (proposed != null) {
    return (
      <div className={`ctx-editor scope-${scope}`}>
        <p className="wb-hint">Preview of the starter template — review and edit before saving.</p>
        <textarea
          className="ctx-textarea"
          value={proposed}
          onChange={(e) => setProposed(e.target.value)}
          rows={16}
        />
        <div className="ctx-actions">
          <button className="wb-link" onClick={() => setProposed(null)}>
            Discard
          </button>
          <button className="wb-cta wb-cta-sm" onClick={onAcceptStarter}>
            Use this
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className={`ctx-editor scope-${scope}`}>
      <div className="ctx-toolbar">
        <span className="ctx-path" title={stat.path}>
          {stat.path}
        </span>
        <span className="spacer" />
        <div className="ctx-mode-toggle">
          <button
            className={mode === "view" ? "active" : ""}
            onClick={() => setMode("view")}
            disabled={mode === "view"}
          >
            view
          </button>
          <button
            className={mode === "edit" ? "active" : ""}
            onClick={() => setMode("edit")}
            disabled={mode === "edit"}
          >
            edit
          </button>
        </div>
        <button
          className="wb-link"
          onClick={() => openExternally(scope).catch((e) => setErr(String(e)))}
          title="Open in external editor"
        >
          open ext
        </button>
      </div>

      {mode === "view" ? (
        content ? (
          <div className="ctx-preview">
            <MarkdownText content={content} />
          </div>
        ) : (
          <p className="wb-hint">(empty)</p>
        )
      ) : (
        <textarea
          className="ctx-textarea"
          value={content}
          onChange={(e) => setContent(e.target.value)}
          rows={16}
          spellCheck={false}
        />
      )}

      {mode === "edit" && (
        <div className="ctx-actions">
          <button
            className="wb-link"
            onClick={() => {
              setContent(original);
              setMode("view");
              setErr(null);
            }}
          >
            Cancel
          </button>
          <button
            className="wb-cta wb-cta-sm"
            onClick={onSave}
            disabled={!dirty || saving}
            title={!dirty ? "No changes" : "Save"}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      )}
      {err && <p className="ctx-error">{err}</p>}
    </div>
  );
}

function MemoryList({ memories }: { memories: MemoryEntry[] }) {
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return memories;
    return memories.filter(
      (m) => m.name.toLowerCase().includes(q) || (m.description ?? "").toLowerCase().includes(q),
    );
  }, [memories, query]);

  if (memories.length === 0) {
    return (
      <p className="wb-hint">
        No memories yet. Claude will save them to{" "}
        <code>~/.claude/projects/&lt;project&gt;/memory/</code> as it learns about you and this
        codebase.
      </p>
    );
  }

  return (
    <>
      <div className="wb-search">
        <input
          className="wb-search-input"
          placeholder="Search memories…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        {query && (
          <button className="wb-search-clear" onClick={() => setQuery("")} title="Clear">
            ×
          </button>
        )}
      </div>
      {filtered.length === 0 ? (
        <p className="wb-hint">No matches.</p>
      ) : (
        <ul className="ctx-memory-list">
          {filtered.map((m) => (
            <MemoryRow
              key={m.name}
              entry={m}
              expanded={expanded === m.name}
              onToggle={() => setExpanded(expanded === m.name ? null : m.name)}
            />
          ))}
        </ul>
      )}
    </>
  );
}

function MemoryRow({
  entry,
  expanded,
  onToggle,
}: {
  entry: MemoryEntry;
  expanded: boolean;
  onToggle: () => void;
}) {
  const readMemory = useContextStore((s) => s.readMemory);
  const deleteMemory = useContextStore((s) => s.deleteMemory);
  const [content, setContent] = useState<string>("");
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!expanded) return;
    let cancelled = false;
    readMemory(entry.name)
      .then((c) => {
        if (!cancelled) setContent(c);
      })
      .catch((e) => {
        if (!cancelled) setErr(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [expanded, entry.name, readMemory]);

  const onDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (entry.isIndex) {
      alert("MEMORY.md is the index — delete the individual memory files instead.");
      return;
    }
    if (
      confirm(
        `Delete memory "${entry.name}"?\n\nThe agent uses this to remember context — this cannot be undone.`,
      )
    ) {
      deleteMemory(entry.name).catch((e2) => alert(`Could not delete: ${e2}`));
    }
  };

  return (
    <li className={`ctx-memory ${expanded ? "open" : ""} ${entry.isIndex ? "index" : ""}`}>
      <div className="ctx-memory-head" onClick={onToggle}>
        <span className="caret">{expanded ? "▾" : "▸"}</span>
        {entry.kind && <span className={`ctx-memory-kind kind-${entry.kind}`}>{entry.kind}</span>}
        {entry.isIndex && <span className="ctx-memory-kind kind-index">index</span>}
        <span className="ctx-memory-name">{entry.name}</span>
        {entry.description && <span className="ctx-memory-desc">{entry.description}</span>}
        <span className="spacer" />
        <span className="ctx-memory-meta">{formatRelative(entry.modifiedMs)}</span>
        {!entry.isIndex && (
          <button className="ctx-memory-delete" onClick={onDelete} title="Delete">
            ×
          </button>
        )}
      </div>
      {expanded && (
        <div className="ctx-memory-body">
          {err && <p className="ctx-error">{err}</p>}
          {!err && <pre className="ctx-memory-content">{content || "(empty)"}</pre>}
        </div>
      )}
    </li>
  );
}

function formatSize(b: number): string {
  if (b < 1024) return `${b}B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)}KB`;
  return `${(b / (1024 * 1024)).toFixed(1)}MB`;
}

function formatRelative(ms: number): string {
  if (!ms) return "";
  const diff = (Date.now() - ms) / 1000;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  if (diff < 7 * 86400) return `${Math.floor(diff / 86400)}d`;
  return new Date(ms).toLocaleDateString();
}
