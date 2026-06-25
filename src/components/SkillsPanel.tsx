import { useEffect, useMemo, useState } from "react";

import type { PromptEvent } from "../stores/skillsStore";
import { useSkillsStore } from "../stores/skillsStore";
import { usePinsStore, pinKey } from "../stores/pinsStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";
import { ipc } from "../ipc/tauri";
import type { Skill } from "../types/domain";
import { MarkdownText } from "./MarkdownText";

type KindFilter = "all" | "skill" | "command" | "agent";

const RECENT_PREVIEW = 5;

export function SkillsPanel() {
  const installed = useSkillsStore((s) => s.installed);
  const recent = useSkillsStore((s) => s.recent);
  const hooks = useSkillsStore((s) => s.hooks);
  const selected = useSkillsStore((s) => s.selected);
  const selectedMd = useSkillsStore((s) => s.selectedMarkdown);
  const refresh = useSkillsStore((s) => s.refresh);
  const install = useSkillsStore((s) => s.install);
  const uninstall = useSkillsStore((s) => s.uninstall);
  const open = useSkillsStore((s) => s.open);
  const restore = useSkillsStore((s) => s.restoreSnapshot);

  const pinned = usePinsStore((s) => s.pinned);
  const togglePin = usePinsStore((s) => s.toggle);

  const [filter, setFilter] = useState<KindFilter>("all");
  const [query, setQuery] = useState("");
  const [integrationOpen, setIntegrationOpen] = useState(false);
  const [activityExpanded, setActivityExpanded] = useState(false);

  useEffect(() => { refresh(); }, [refresh]);

  // Frequency map from observed prompt events for "recent" boost.
  const frequency = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of recent) {
      if (!e.skill) continue;
      m.set(e.skill, (m.get(e.skill) ?? 0) + 1);
    }
    return m;
  }, [recent]);

  const counts = useMemo(() => ({
    all: installed.length,
    skill: installed.filter((s) => s.kind === "skill").length,
    command: installed.filter((s) => s.kind === "command").length,
    agent: installed.filter((s) => s.kind === "agent").length,
  }), [installed]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = installed.filter((sk) => {
      if (filter !== "all" && sk.kind !== filter) return false;
      if (!q) return true;
      return sk.name.toLowerCase().includes(q) || (sk.description ?? "").toLowerCase().includes(q);
    });
    // Sort: pinned first, then by usage frequency (desc), then alpha by name.
    return list.slice().sort((a, b) => {
      const pa = pinned.has(pinKey(a.kind, a.source, a.name)) ? 1 : 0;
      const pb = pinned.has(pinKey(b.kind, b.source, b.name)) ? 1 : 0;
      if (pa !== pb) return pb - pa;
      const fa = frequency.get(a.name) ?? 0;
      const fb = frequency.get(b.name) ?? 0;
      if (fa !== fb) return fb - fa;
      return a.name.localeCompare(b.name);
    });
  }, [installed, filter, query, pinned, frequency]);

  if (selected) {
    return <SkillDetail skill={selected} md={selectedMd} onBack={() => open(null)} />;
  }

  const integrationActive = !!hooks?.installed;
  const visibleRecent = activityExpanded ? recent : recent.slice(0, RECENT_PREVIEW);

  return (
    <div className="workbench">
      <div className="workbench-header workbench-header-slim">
        <span className="workbench-title">skills</span>
        <span className="spacer" />
        <button className="workbench-action" onClick={refresh} title="Refresh">↻</button>
      </div>

      <div className="workbench-body">
        {/* Integration: compact pill, expandable */}
        <section className="wb-section wb-integration">
          <button
            className="wb-integration-header"
            onClick={() => setIntegrationOpen((v) => !v)}
            title={integrationOpen ? "Hide details" : "Show details"}
          >
            <span className="caret">{integrationOpen ? "▾" : "▸"}</span>
            <span>integration</span>
            <span className={`wb-status ${integrationActive ? "ok" : "off"}`}>
              {integrationActive ? "active" : "inactive"}
            </span>
          </button>
          {integrationOpen && (
            <div className="wb-integration-body">
              {integrationActive ? (
                <p className="wb-hint">
                  Hook installed. Claude sessions inside this terminal trigger
                  snapshots and feed the activity stream below.
                  <button className="wb-link" onClick={uninstall}>disable</button>
                </p>
              ) : (
                <p className="wb-hint">
                  Snapshots + activity tracking require a small hook in
                  {" "}<code>~/.claude/settings.json</code>. It only activates when claude runs
                  inside Agent Console (gated by env var).
                  <button className="wb-cta" onClick={install}>enable</button>
                </p>
              )}
            </div>
          )}
        </section>

        {/* Recent activity */}
        <section className="wb-section">
          <div className="wb-section-title">
            recent activity
            {recent.length > 0 && <span className="wb-count">{recent.length}</span>}
          </div>
          {recent.length === 0 ? (
            <p className="wb-hint">
              {integrationActive
                ? "Nothing yet. Run claude in the terminal and ask it something."
                : "Enable integration above to start observing prompts."}
            </p>
          ) : (
            <>
              <ul className="wb-events">
                {visibleRecent.map((e) => <EventRow key={e.id} event={e} onRestore={restore} />)}
              </ul>
              {recent.length > RECENT_PREVIEW && (
                <button
                  className="wb-link wb-show-more"
                  onClick={() => setActivityExpanded((v) => !v)}
                >
                  {activityExpanded ? "show less" : `show ${recent.length - RECENT_PREVIEW} more`}
                </button>
              )}
            </>
          )}
        </section>

        {/* Installed: search + filter chips + list */}
        <section className="wb-section">
          <div className="wb-section-title">
            installed
            <span className="wb-count">{installed.length}</span>
          </div>

          {installed.length > 0 && (
            <>
              <div className="wb-search">
                <input
                  className="wb-search-input"
                  placeholder="Search skills…"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
                {query && (
                  <button
                    className="wb-search-clear"
                    onClick={() => setQuery("")}
                    title="Clear"
                  >×</button>
                )}
              </div>

              <div className="wb-chips">
                <KindChip label="All" value="all" active={filter === "all"} count={counts.all} onClick={setFilter} />
                <KindChip label="Skills" value="skill" active={filter === "skill"} count={counts.skill} onClick={setFilter} />
                <KindChip label="Commands" value="command" active={filter === "command"} count={counts.command} onClick={setFilter} />
                <KindChip label="Agents" value="agent" active={filter === "agent"} count={counts.agent} onClick={setFilter} />
              </div>
            </>
          )}

          {installed.length === 0 ? (
            <div className="wb-empty">
              <p className="wb-hint">
                No skills yet. Skills live in
                {" "}<code>.claude/skills</code>, <code>.claude/commands</code>, and
                {" "}<code>.claude/agents</code>, and are invoked from Claude in the terminal.
              </p>
              <p className="wb-hint">
                Fastest way in? Open the <strong>Advisor</strong> tab — it analyzes
                your project and proposes concrete skills in one click.
              </p>
            </div>
          ) : filtered.length === 0 ? (
            <p className="wb-hint">No matches for the current filter.</p>
          ) : (
            <ul className="wb-skill-list">
              {filtered.map((sk) => {
                const key = pinKey(sk.kind, sk.source, sk.name);
                return (
                  <SkillRow
                    key={`${sk.source}-${sk.kind}-${sk.name}`}
                    skill={sk}
                    pinned={pinned.has(key)}
                    onClick={() => open(sk)}
                    onTogglePin={() => togglePin(key)}
                  />
                );
              })}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

function KindChip({ label, value, active, count, onClick }: {
  label: string;
  value: KindFilter;
  active: boolean;
  count: number;
  onClick: (v: KindFilter) => void;
}) {
  return (
    <button
      className={`wb-chip ${active ? "active" : ""}`}
      onClick={() => onClick(value)}
      disabled={count === 0 && value !== "all"}
    >
      {label}
      <span className="wb-chip-count">{count}</span>
    </button>
  );
}

function SkillRow({ skill, pinned, onClick, onTogglePin }: {
  skill: Skill;
  pinned: boolean;
  onClick: () => void;
  onTogglePin: () => void;
}) {
  const snippet = invocationSnippet(skill);
  const onInsert = async (e: React.MouseEvent) => {
    e.stopPropagation();
    await insertIntoTerminal(snippet);
  };
  const onPin = (e: React.MouseEvent) => {
    e.stopPropagation();
    onTogglePin();
  };
  return (
    <li className={`wb-skill ${pinned ? "pinned" : ""}`} onClick={onClick} title={skill.path}>
      <span className={`wb-skill-kind kind-${skill.kind}`}>{kindLabel(skill.kind)}</span>
      <div className="wb-skill-text">
        <div className="wb-skill-name">
          {skill.name}
          {skill.source === "user" && <span className="wb-skill-source">user</span>}
        </div>
        {skill.description && <div className="wb-skill-desc">{skill.description}</div>}
      </div>
      <div className="wb-skill-actions">
        <button
          className={`wb-skill-pin ${pinned ? "active" : ""}`}
          onClick={onPin}
          title={pinned ? "Unpin" : "Pin to top"}
        >{pinned ? "★" : "☆"}</button>
        <button
          className="wb-skill-invoke"
          onClick={onInsert}
          title={`Insert into terminal: ${snippet.trim()}`}
        >→</button>
      </div>
    </li>
  );
}

/// Returns the text to inject into the active terminal session for this skill.
function invocationSnippet(skill: Skill): string {
  if (skill.kind === "command") return `/${skill.name} `;
  if (skill.kind === "agent") return `Use the ${skill.name} agent to `;
  return `Use the ${skill.name} skill to `;
}

async function insertIntoTerminal(text: string): Promise<void> {
  const { activeId } = useTerminalsStore.getState();
  useUIStore.getState().setTab("terminal");
  if (!activeId) {
    // Fallback: copy so the user can paste manually.
    try { await navigator.clipboard.writeText(text); } catch { /* ignore */ }
    return;
  }
  try { await ipc.termWrite(activeId, text); } catch { /* ignore */ }
}

function EventRow({ event, onRestore }: {
  event: PromptEvent;
  onRestore: (sha: string) => void;
}) {
  const time = new Date(event.ts).toLocaleTimeString();
  return (
    <li className="wb-event">
      <span className="wb-event-time">{time}</span>
      {event.skill && <span className="wb-event-skill">/{event.skill}</span>}
      <span className="wb-event-prompt">{truncate(event.prompt, 120)}</span>
      {event.snapshotCommitSha && (
        <button
          className="wb-event-restore"
          title="Restore the working tree to before this turn (a backup is taken first)"
          onClick={() => {
            if (event.snapshotCommitSha && confirm(
              "Restore the working tree to before this turn?\n\n" +
              "This discards ALL changes made after this point — not just this turn's. " +
              "A backup is taken first, so you can undo from the command palette.",
            )) {
              onRestore(event.snapshotCommitSha);
            }
          }}
        >↶</button>
      )}
    </li>
  );
}

function SkillDetail({ skill, md, onBack }: { skill: Skill; md: string; onBack: () => void }) {
  const snippet = invocationSnippet(skill);
  const [copied, setCopied] = useState(false);

  const onCopy = () => {
    navigator.clipboard?.writeText(snippet.trim()).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    }).catch(() => {});
  };
  const onInsert = () => { insertIntoTerminal(snippet); };

  return (
    <div className="workbench">
      <div className="workbench-header">
        <button className="workbench-action" onClick={onBack}>← back</button>
        <span className="workbench-title wb-detail-name">{skill.name}</span>
        <span className="spacer" />
        <span className={`wb-skill-kind kind-${skill.kind}`}>{kindLabel(skill.kind)}</span>
      </div>
      <div className="workbench-body">
        {skill.description && <div className="wb-detail-desc">{skill.description}</div>}
        <div className="wb-detail-invoke">
          <code>{snippet.trim()}</code>
          <button className="wb-cta wb-cta-sm" onClick={onInsert}>insert →</button>
          <button className="wb-link" onClick={onCopy}>{copied ? "copied!" : "copy"}</button>
        </div>
        <div className="wb-detail-meta">
          <span className="wb-detail-source">source: {skill.source}</span>
          <span className="wb-detail-path" title={skill.path}>{skill.path}</span>
        </div>
        {skill.allowedTools.length > 0 && (
          <div className="wb-detail-tools">
            <span className="wb-section-title" style={{ display: "block", marginBottom: 4 }}>allowed tools</span>
            {skill.allowedTools.map((t) => <span key={t} className="wb-tool-tag">{t}</span>)}
          </div>
        )}
        <div className="wb-detail-md">
          {md ? <MarkdownText content={md} /> : <span className="wb-hint">Loading…</span>}
        </div>
      </div>
    </div>
  );
}

function kindLabel(kind: Skill["kind"]): string {
  switch (kind) {
    case "skill": return "S";
    case "command": return "/";
    case "agent": return "A";
  }
}

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n) + "…" : s;
}
