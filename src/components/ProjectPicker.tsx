import { useEffect } from "react";

import { useSessionStore } from "../stores/sessionStore";
import { useProjectsStore } from "../stores/projectsStore";
import { useThemeStore } from "../stores/themeStore";
import { pickFolder } from "../ipc/tauri";
import { Icon } from "./Icon";

export function ProjectPicker() {
  const { openProject, loading, error } = useSessionStore();
  const recent = useProjectsStore((s) => s.recent);
  const loadRecent = useProjectsStore((s) => s.load);
  const forget = useProjectsStore((s) => s.forget);

  const theme = useThemeStore((s) => s.theme);
  const toggleTheme = useThemeStore((s) => s.toggle);

  useEffect(() => { loadRecent(); }, [loadRecent]);

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
        <p>
          A minimalist, AI-native console for directing agents inside a repository.
        </p>
        <button className="primary" onClick={onPick} disabled={loading}>
          {loading ? "Opening..." : "Open folder"}
        </button>
        {error && <div className="error">{error}</div>}

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
                >×</button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
