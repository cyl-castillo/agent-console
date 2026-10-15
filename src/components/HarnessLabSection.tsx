import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useHarnessLabStore } from "../stores/harnessLabStore";
import { useSessionStore } from "../stores/sessionStore";
import type { HeldOutTask } from "../types/domain";

/// D12 in the Learning panel: the one declared metric, the editable
/// allowlist, the budget, the hidden tasks (kept outside the repo, edited
/// only here), and the experiments with their lineage and PR links. The
/// console proposes; the human merges.
export function HarnessLabSection() {
  const projectRoot = useSessionStore((s) => s.project?.root);
  const summary = useHarnessLabStore((s) => s.summary);
  const running = useHarnessLabStore((s) => s.running);
  const error = useHarnessLabStore((s) => s.error);
  const load = useHarnessLabStore((s) => s.load);
  const saveConfig = useHarnessLabStore((s) => s.saveConfig);
  const saveHeldout = useHarnessLabStore((s) => s.saveHeldout);
  const runNow = useHarnessLabStore((s) => s.runNow);
  const [editable, setEditable] = useState("");
  const [focus, setFocus] = useState("");
  const [tasks, setTasks] = useState<HeldOutTask[]>([]);
  const [showTasks, setShowTasks] = useState(false);

  useEffect(() => {
    if (projectRoot) void load(projectRoot);
  }, [projectRoot, load]);
  useEffect(() => {
    setEditable(summary?.config.editable.join("\n") ?? "");
    setFocus(summary?.config.focus ?? "");
    setTasks(summary?.heldout.tasks ?? []);
  }, [summary]);

  if (!summary) return null;
  const cfg = summary.config;
  const exps = [...summary.experiments].reverse().slice(0, 12);

  return (
    <section className="wb-section" data-testid="harness-lab">
      <div className="wb-section-title">harness lab</div>
      <p className="wb-hint">
        The console improves its own harness (CLAUDE.md, skills, agents, hooks) in a branch per
        experiment and measures it on tasks the agent never sees. Metric <code>{cfg.metric}</code>,
        computed by the console. Kept experiments become PRs; nothing merges by itself.
      </p>
      <div className="wb-field-row" style={{ gap: 12, flexWrap: "wrap" }}>
        <span className="wb-hint" data-testid="lab-stats">
          {summary.experiments.length} experiments · {summary.kept} kept · {summary.clean_runs}{" "}
          without touching the evaluator · {summary.heldout.tasks.length} hidden task
          {summary.heldout.tasks.length === 1 ? "" : "s"} (v{summary.heldout.version})
        </span>
        <button
          className="wb-cta wb-cta-sm"
          disabled={running || summary.heldout.tasks.length === 0}
          onClick={() => void runNow()}
          title={
            summary.heldout.tasks.length === 0
              ? "Declare hidden tasks first"
              : "Run one experiment now (a worktree, one harness edit, the hidden window twice)"
          }
        >
          {running ? "running…" : "Run experiment"}
        </button>
      </div>
      {error && <div className="rt-banner rt-banner-error">{error}</div>}

      <label className="wb-field">
        <span className="wb-field-label">
          editable paths (one per line; everything else is rejected)
        </span>
        <textarea
          className="wb-input"
          rows={3}
          value={editable}
          onChange={(e) => setEditable(e.target.value)}
          onBlur={() => {
            const list = editable
              .split("\n")
              .map((s) => s.trim())
              .filter(Boolean);
            if (list.join("\n") !== cfg.editable.join("\n")) void saveConfig({ editable: list });
          }}
        />
      </label>
      <label className="wb-field">
        <span className="wb-field-label">focus for the agent (optional)</span>
        <input
          className="wb-input"
          value={focus}
          placeholder="e.g. the agent keeps forgetting to run prettier"
          onChange={(e) => setFocus(e.target.value)}
          onBlur={() => focus !== cfg.focus && void saveConfig({ focus })}
        />
      </label>
      <div className="wb-field-row">
        <label className="wb-field">
          <span className="wb-field-label">budget: minutes of silence</span>
          <input
            className="wb-input wb-input-narrow"
            type="number"
            min={1}
            value={cfg.budget.idleMinutes}
            onChange={(e) =>
              void saveConfig({
                budget: { ...cfg.budget, idleMinutes: Math.max(1, Number(e.target.value) || 1) },
              })
            }
          />
        </label>
        <label className="wb-field">
          <span className="wb-field-label">max tokens</span>
          <input
            className="wb-input wb-input-narrow"
            type="number"
            min={1000}
            step={1000}
            value={cfg.budget.maxTokens}
            onChange={(e) =>
              void saveConfig({
                budget: {
                  ...cfg.budget,
                  maxTokens: Math.max(1000, Number(e.target.value) || 1000),
                },
              })
            }
          />
        </label>
      </div>

      <button className="wb-link" onClick={() => setShowTasks((v) => !v)}>
        {showTasks ? "hide" : "edit"} hidden tasks ({tasks.length})
      </button>
      {showTasks && (
        <div className="wb-field" data-testid="lab-heldout">
          <p className="wb-hint">
            Stored outside the repository; the agent never reads them. Each task: a prompt the agent
            gets under the candidate harness, and a shell check (exit 0 = pass).
          </p>
          {tasks.map((t, i) => (
            <div key={i} className="wb-field-row" style={{ alignItems: "flex-start" }}>
              <input
                className="wb-input"
                placeholder="prompt"
                value={t.prompt}
                onChange={(e) =>
                  setTasks(tasks.map((x, j) => (j === i ? { ...x, prompt: e.target.value } : x)))
                }
              />
              <input
                className="wb-input"
                placeholder="check (sh -c)"
                value={t.check}
                onChange={(e) =>
                  setTasks(tasks.map((x, j) => (j === i ? { ...x, check: e.target.value } : x)))
                }
              />
              <button className="wb-link" onClick={() => setTasks(tasks.filter((_, j) => j !== i))}>
                ×
              </button>
            </div>
          ))}
          <div className="wb-field-row">
            <button
              className="wb-link"
              onClick={() =>
                setTasks([...tasks, { id: `t${Date.now().toString(36)}`, prompt: "", check: "" }])
              }
            >
              + task
            </button>
            <button className="wb-cta wb-cta-sm" onClick={() => void saveHeldout(tasks)}>
              Save as v{summary.heldout.version + 1}
            </button>
          </div>
        </div>
      )}

      {exps.length > 0 && (
        <ul className="sessions-list" data-testid="lab-experiments">
          {exps.map((e) => (
            <li key={e.id} className="session-row" title={e.stopReason ?? e.hypothesis}>
              <span className={`rt-stage rt-stage-${e.status}`}>{e.status}</span>
              <span className="session-name">
                {e.hypothesis || e.id}
                {e.parentId ? ` ↳ ${e.parentId}` : ""}
              </span>
              <span className="session-meta">
                {e.score ? `${e.score.passed}/${e.score.total}` : "—"}
                {e.baseline ? ` vs ${e.baseline.passed}/${e.baseline.total}` : ""}
                {e.outsideFiles.length > 0 ? " · touched outside" : ""}
              </span>
              {e.prUrl && (
                <button
                  className="wb-link"
                  onClick={() => void openUrl(e.prUrl!)}
                  title="Open the pull request (you merge, the console never does)"
                >
                  PR
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
