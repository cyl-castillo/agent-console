import { useEffect, useMemo, useState } from "react";

import { useContextStore } from "../stores/contextStore";
import { useInjectStore } from "../stores/injectStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { typeIntoActiveSession } from "../lib/termInput";
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

        <SemanticSearch />

        <MemoryInjectionSection />

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

/// Memory injection (E1): the toggle for "feed relevant memories into every
/// prompt" plus the audit trail of what was recently injected. Transparency is
/// the contract — this list is why the feature is trustworthy.
function MemoryInjectionSection() {
  const project = useSessionStore((s) => s.project);
  const enabled = useInjectStore((s) => s.enabled);
  const recent = useInjectStore((s) => s.recent);
  const setEnabled = useInjectStore((s) => s.setEnabled);
  const feedback = useInjectStore((s) => s.feedback);
  const vote = useInjectStore((s) => s.vote);
  const resetVerdicts = useInjectStore((s) => s.resetVerdicts);
  const [open, setOpen] = useState(false);

  if (!project) return null;
  const forProject = recent.filter((r) => r.projectRoot === project.root);
  // Corpus outcome stats, most-used first (backend order), verdicted or
  // excluded docs always shown — exclusions must never be invisible.
  const statsList = Object.values(feedback).filter(
    (d) => d.injectedCount > 0 || d.helpful > 0 || d.unhelpful > 0,
  );

  return (
    <section className="wb-section">
      <button className="ctx-section-head scope-project" onClick={() => setOpen((v) => !v)}>
        <span className="caret">{open ? "▾" : "▸"}</span>
        <span className="ctx-scope-badge scope-project">◈</span>
        <span className="ctx-section-title">Memory injection</span>
        <span className="ctx-section-meta">{enabled ? "on" : "off"}</span>
      </button>
      {open && (
        <div className="inject-body">
          <label className="inject-toggle">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => void setEnabled(project.root, e.target.checked)}
            />
            <span>
              Feed relevant memories into every prompt (both engines). Retrieval is local; the
              status bar shows a ◈ chip whenever something was injected.
            </span>
          </label>
          {forProject.length === 0 ? (
            <p className="wb-hint">
              Nothing injected yet. Injection needs the semantic index (run a search or ⟳ above
              once) and kicks in when a prompt clearly matches a saved memory or skill.
            </p>
          ) : (
            <ul className="inject-list">
              {forProject.map((r, i) => (
                <li key={i} className="inject-row">
                  <span className="inject-when">{formatRelative(r.tsMs)}</span>
                  <span className="inject-prompt" title={r.promptHead}>
                    “{r.promptHead}”
                  </span>
                  {r.hits.map((h) => (
                    <span key={h.id} className="inject-hits">
                      {h.title} ({Math.round(h.score * 100)}%)
                      <button
                        className="inject-vote-btn"
                        title="Useful — ranks higher for future injections"
                        onClick={() => void vote(project.root, h.id, true)}
                      >
                        👍{feedback[h.id]?.helpful ? ` ${feedback[h.id].helpful}` : ""}
                      </button>
                      <button
                        className="inject-vote-btn"
                        title="Got in the way — 3× without a 👍 excludes it from injection"
                        onClick={() => void vote(project.root, h.id, false)}
                      >
                        👎{feedback[h.id]?.unhelpful ? ` ${feedback[h.id].unhelpful}` : ""}
                      </button>
                      {feedback[h.id]?.excluded && (
                        <span
                          className="inject-excluded"
                          title="No longer injected (still searchable)"
                        >
                          excluded
                        </span>
                      )}
                    </span>
                  ))}
                </li>
              ))}
            </ul>
          )}
          {statsList.length > 0 && (
            <>
              <div className="inject-stats-head">Corpus by usefulness</div>
              <ul className="inject-list">
                {statsList.map((d) => (
                  <li key={d.docId} className="inject-row inject-stat-row">
                    <span className="inject-prompt" title={d.docId}>
                      {d.docId.replace(/^(memory|skill):/, "")}
                    </span>
                    <span className="inject-hits">
                      {d.injectedCount}× injected · 👍{d.helpful} · 👎{d.unhelpful}
                      {d.excluded && (
                        <>
                          <span className="inject-excluded">excluded</span>
                          <button
                            className="inject-vote-btn"
                            title="Clear verdicts and let this doc be injected again"
                            onClick={() => void resetVerdicts(project.root, d.docId)}
                          >
                            restore
                          </button>
                        </>
                      )}
                    </span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </section>
  );
}

/// Semantic (embedding) search over the project's memories and skills.
/// Local model, local index — nothing leaves the machine.
function SemanticSearch() {
  const searchHits = useContextStore((s) => s.searchHits);
  const searching = useContextStore((s) => s.searching);
  const searchError = useContextStore((s) => s.searchError);
  const semanticSearch = useContextStore((s) => s.semanticSearch);
  const clearSearch = useContextStore((s) => s.clearSearch);
  const reindexing = useContextStore((s) => s.reindexing);
  const semanticReindex = useContextStore((s) => s.semanticReindex);
  const readMemory = useContextStore((s) => s.readMemory);
  const showToast = useToastStore((s) => s.show);
  const [query, setQuery] = useState("");
  const [everSearched, setEverSearched] = useState(false);

  const run = () => {
    if (!query.trim() || searching) return;
    setEverSearched(true);
    void semanticSearch(query);
  };

  const sendToComposer = async (hit: (typeof searchHits)[number]) => {
    if (hit.kind !== "memory") return;
    const name = hit.id.replace(/^memory:/, "");
    try {
      const content = await readMemory(name);
      const clipped = content.length > 1500 ? `${content.slice(0, 1500)}…` : content;
      await typeIntoActiveSession(`Relevant saved memory (${name}):\n${clipped}\n`);
      showToast("Memory typed into the active session — review and send", "success");
    } catch (e) {
      showToast(`Couldn't load memory: ${String(e).slice(0, 120)}`, "error");
    }
  };

  return (
    <section className="wb-section">
      <div className="ctx-search-row">
        <input
          className="ctx-search-input"
          value={query}
          placeholder="Search memories & skills by meaning…"
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") run();
            if (e.key === "Escape") {
              setQuery("");
              clearSearch();
              setEverSearched(false);
            }
          }}
        />
        <button
          className="wb-cta wb-cta-sm"
          onClick={run}
          disabled={searching || !query.trim()}
          title="Semantic search (local embeddings — nothing leaves this machine)"
        >
          {searching ? "…" : "Search"}
        </button>
        <button
          className="workbench-action"
          onClick={async () => {
            const summary = await semanticReindex();
            if (summary) showToast(`Semantic index: ${summary}`, "success");
          }}
          disabled={reindexing}
          title="Rebuild the semantic index (incremental — only changed items re-embed)"
        >
          {reindexing ? "…" : "⟳"}
        </button>
      </div>
      {(searching || reindexing) && (
        <div className="wb-hint">
          Working… the first run downloads a small local model (~100 MB), which can take a minute.
        </div>
      )}
      {searchError && <PanelError message={searchError} />}
      {everSearched && !searching && searchHits.length === 0 && !searchError && (
        <div className="wb-hint">No matches.</div>
      )}
      {searchHits.length > 0 && (
        <ul className="ctx-search-hits">
          {searchHits.map((h) => (
            <li key={h.id} className="ctx-search-hit">
              <div className="ctx-hit-top">
                <span className={`ctx-hit-kind kind-${h.kind}`}>{h.kind}</span>
                <span className="ctx-hit-title" title={h.title}>
                  {h.title}
                </span>
                <span className="ctx-hit-score" title={`similarity ${h.score.toFixed(3)}`}>
                  {(h.score * 100).toFixed(0)}%
                </span>
              </div>
              <div className="ctx-hit-snippet">{h.snippet}</div>
              {h.kind === "memory" && (
                <button
                  className="wb-link"
                  onClick={() => void sendToComposer(h)}
                  title="Type this memory into the active session's input (you review, then send)"
                >
                  ▸ to composer
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
