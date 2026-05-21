import { useEffect, useMemo } from "react";

import type { PromptEvent } from "../stores/skillsStore";
import { useSkillsStore } from "../stores/skillsStore";
import type { Skill } from "../types/domain";

/// Right-panel workbench: hooks status + installed skills + recent activity.
/// Replaces the embedded chat — the agent now lives in the terminal.
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

  useEffect(() => { refresh(); }, [refresh]);

  const grouped = useMemo(() => groupBySource(installed), [installed]);

  if (selected) {
    return <SkillDetail skill={selected} md={selectedMd} onBack={() => open(null)} />;
  }

  return (
    <div className="workbench">
      <div className="workbench-header">
        <span>workbench</span>
        <span className="spacer" />
        <button className="workbench-action" onClick={refresh} title="Refresh">↻</button>
      </div>

      <div className="workbench-body">
        {/* Hooks integration banner */}
        <section className="wb-section">
          <div className="wb-section-title">
            integration
            <span className={`wb-status ${hooks?.installed ? "ok" : "off"}`}>
              {hooks?.installed ? "active" : "inactive"}
            </span>
          </div>
          {hooks?.installed ? (
            <p className="wb-hint">
              Hook installed. Claude sessions inside this terminal will trigger
              snapshots and feed the activity stream below.
              <button className="wb-link" onClick={uninstall}>disable</button>
            </p>
          ) : (
            <p className="wb-hint">
              Snapshots + activity tracking require a small hook in
              <code>~/.claude/settings.json</code>. It only activates when claude runs
              inside Agent Console (gated by env var).
              <button className="wb-cta" onClick={install}>enable integration</button>
            </p>
          )}
        </section>

        {/* Recent activity */}
        <section className="wb-section">
          <div className="wb-section-title">recent activity</div>
          {recent.length === 0 ? (
            <p className="wb-hint">
              {hooks?.installed
                ? "Nothing yet. Run claude in the terminal and ask it something."
                : "Enable integration above to start observing prompts."}
            </p>
          ) : (
            <ul className="wb-events">
              {recent.map((e) => <EventRow key={e.id} event={e} onRestore={restore} />)}
            </ul>
          )}
        </section>

        {/* Installed skills / commands / agents */}
        <section className="wb-section">
          <div className="wb-section-title">
            installed
            <span className="wb-count">{installed.length}</span>
          </div>
          {installed.length === 0 ? (
            <p className="wb-hint">
              No skills, commands, or agents found in <code>.claude/skills</code>,
              <code>.claude/commands</code>, <code>.claude/agents</code> (project or user).
            </p>
          ) : (
            <div className="wb-skill-groups">
              {grouped.map(({ source, items }) => (
                <div key={source} className="wb-skill-group">
                  <div className="wb-group-label">{source}</div>
                  <ul className="wb-skill-list">
                    {items.map((sk) => (
                      <SkillRow key={`${sk.source}-${sk.kind}-${sk.name}`} skill={sk} onClick={() => open(sk)} />
                    ))}
                  </ul>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Marketplace placeholder */}
        <section className="wb-section wb-section-muted">
          <div className="wb-section-title">marketplace</div>
          <p className="wb-hint">
            Curated skill index — coming in v0.4. For now, add skills manually by
            dropping them into <code>.claude/skills/&lt;name&gt;/SKILL.md</code>.
          </p>
        </section>
      </div>
    </div>
  );
}

function groupBySource(skills: Skill[]): Array<{ source: string; items: Skill[] }> {
  const map = new Map<string, Skill[]>();
  for (const sk of skills) {
    if (!map.has(sk.source)) map.set(sk.source, []);
    map.get(sk.source)!.push(sk);
  }
  return Array.from(map.entries()).map(([source, items]) => ({ source, items }));
}

function SkillRow({ skill, onClick }: { skill: Skill; onClick: () => void }) {
  return (
    <li className="wb-skill" onClick={onClick} title={skill.path}>
      <span className={`wb-skill-kind kind-${skill.kind}`}>{skill.kind[0].toUpperCase()}</span>
      <div className="wb-skill-text">
        <div className="wb-skill-name">{skill.name}</div>
        {skill.description && <div className="wb-skill-desc">{skill.description}</div>}
      </div>
    </li>
  );
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
          title="Restore working tree to before this turn"
          onClick={() => {
            if (event.snapshotCommitSha && confirm("Restore to before this turn? Uncommitted changes will be lost.")) {
              onRestore(event.snapshotCommitSha);
            }
          }}
        >↶</button>
      )}
    </li>
  );
}

function SkillDetail({ skill, md, onBack }: { skill: Skill; md: string; onBack: () => void }) {
  return (
    <div className="workbench">
      <div className="workbench-header">
        <button className="workbench-action" onClick={onBack}>← back</button>
        <span className="spacer" />
        <span className={`wb-skill-kind kind-${skill.kind}`}>{skill.kind}</span>
      </div>
      <div className="workbench-body">
        <div className="wb-detail-title">{skill.name}</div>
        {skill.description && <div className="wb-detail-desc">{skill.description}</div>}
        <div className="wb-detail-path">{skill.path}</div>
        {skill.allowedTools.length > 0 && (
          <div className="wb-detail-tools">
            <span className="wb-section-title" style={{ display: "block", marginBottom: 4 }}>allowed tools</span>
            {skill.allowedTools.map((t) => <span key={t} className="wb-tool-tag">{t}</span>)}
          </div>
        )}
        <pre className="wb-detail-md">{md || "Loading…"}</pre>
      </div>
    </div>
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n) + "…" : s;
}
