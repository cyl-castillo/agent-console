import { useEffect } from "react";

import { useSessionStore } from "../stores/sessionStore";
import { useProjectsStore } from "../stores/projectsStore";
import { useThemeStore } from "../stores/themeStore";
import { usePreflightStore, toolStatus } from "../stores/preflightStore";
import type { PreflightTool } from "../types/domain";
import { pickFolder } from "../ipc/tauri";
import { Icon } from "./Icon";

function ToolPill({ t, label }: { t?: PreflightTool; label: string }) {
  if (!t) return null;
  return (
    <span
      className={`preflight-pill ${t.found ? "found" : "missing"}`}
      title={t.found ? (t.version ?? "found") : "not found on your PATH"}
    >
      {t.found ? "✓" : "✕"} {label}
    </span>
  );
}

// First-run environment check — surfaces a missing Claude CLI / Node *before*
// the user opens a repo and hits a cryptic `command not found` in the terminal.
function PreflightCard() {
  const result = usePreflightStore((s) => s.result);
  const checking = usePreflightStore((s) => s.checking);
  const check = usePreflightStore((s) => s.check);

  useEffect(() => {
    void check();
  }, [check]);

  if (!result) {
    return checking ? <div className="preflight checking">Checking your setup…</div> : null;
  }

  const claude = toolStatus(result, "claude");
  const node = toolStatus(result, "node");
  const git = toolStatus(result, "git");
  const claudeMissing = claude && !claude.found;
  const nodeMissing = node && !node.found;
  const tone = claudeMissing ? "bad" : nodeMissing ? "warn" : "ok";

  return (
    <div className={`preflight ${tone}`}>
      {claudeMissing ? (
        <div className="preflight-msg">
          <div className="preflight-title">Claude CLI not found</div>
          <div className="preflight-body">
            Agent Console drives the Claude CLI. Install it, then sign in once:
            <pre>npm install -g @anthropic-ai/claude-code{"\n"}claude # sign in</pre>
          </div>
        </div>
      ) : nodeMissing ? (
        <div className="preflight-msg">
          <div className="preflight-title">Node.js not found</div>
          <div className="preflight-body">
            The per-tool approval hook is a Node script — install Node ≥ 20 on your PATH.
          </div>
        </div>
      ) : (
        <div className="preflight-title ok">Setup ready</div>
      )}

      <div className="preflight-row">
        <div className="preflight-tools">
          <ToolPill t={claude} label="Claude CLI" />
          <ToolPill t={node} label="Node" />
          <ToolPill t={git} label="git" />
        </div>
        <button className="btn btn-link btn-sm" onClick={() => void check()} disabled={checking}>
          {checking ? "Checking…" : "Re-check"}
        </button>
      </div>
    </div>
  );
}

export function ProjectPicker() {
  const { openProject, loading, error } = useSessionStore();
  const recent = useProjectsStore((s) => s.recent);
  const loadRecent = useProjectsStore((s) => s.load);
  const forget = useProjectsStore((s) => s.forget);

  const theme = useThemeStore((s) => s.theme);
  const toggleTheme = useThemeStore((s) => s.toggle);

  useEffect(() => {
    loadRecent();
  }, [loadRecent]);

  const onPick = async () => {
    const path = await pickFolder();
    if (path) await openProject(path);
  };

  return (
    <div className="picker">
      <button
        className="picker-theme-toggle"
        onClick={toggleTheme}
        title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} size={16} />
      </button>
      <div className="picker-card">
        <h1>AGENT CONSOLE</h1>
        <p>A minimalist, AI-native console for directing agents inside a repository.</p>
        <p className="picker-sub">
          Pick a git repository — the agent works inside it. You approve every file change and
          command, and any turn can be rewound.
        </p>
        <button className="btn btn-primary" onClick={onPick} disabled={loading}>
          {loading ? "Opening..." : "Open folder"}
        </button>
        {error && <div className="error">{error}</div>}

        <PreflightCard />

        {recent.length > 0 && (
          <div className="recent">
            <div className="recent-title">Recent</div>
            {recent.map((r) => (
              <div key={r.path} className="recent-row">
                <button
                  className="recent-open"
                  onClick={() => openProject(r.path)}
                  disabled={loading}
                  title={r.path}
                >
                  <span className="recent-name">{r.name}</span>
                  <span className="recent-path">{r.path}</span>
                </button>
                <button
                  className="recent-forget"
                  onClick={() => forget(r.path)}
                  title="Remove from list"
                  aria-label="Remove from list"
                >
                  <Icon name="x" size={14} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
