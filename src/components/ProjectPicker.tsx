import { useEffect } from "react";

import { useSessionStore } from "../stores/sessionStore";
import { useProjectsStore } from "../stores/projectsStore";
import { pickFolder } from "../ipc/tauri";

export function ProjectPicker() {
  const { openProject, loading, error } = useSessionStore();
  const recent = useProjectsStore((s) => s.recent);
  const loadRecent = useProjectsStore((s) => s.load);
  const forget = useProjectsStore((s) => s.forget);

  useEffect(() => { loadRecent(); }, [loadRecent]);

  const onPick = async () => {
    const path = await pickFolder();
    if (path) await openProject(path);
  };

  return (
    <div className="picker">
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
