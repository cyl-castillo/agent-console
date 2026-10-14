import type { Project } from "../types/domain";
import { Icon } from "../components/Icon";
import { useChangesStore } from "../stores/changesStore";
import { useOverlaysStore } from "../stores/overlaysStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useThemeStore } from "../stores/themeStore";
import { useUIStore } from "../stores/uiStore";
import { copyProjectPath } from "./actions";

export function TopBar({ project }: { project: Project }) {
  const changesCount = useChangesStore((s) => s.status?.changes.length ?? 0);
  const leftOpen = useUIStore((s) => s.leftOpen);
  const rightOpen = useUIStore((s) => s.rightOpen);
  const toggleLeft = useUIStore((s) => s.toggleLeft);
  const toggleRight = useUIStore((s) => s.toggleRight);
  const theme = useThemeStore((s) => s.theme);
  const toggleTheme = useThemeStore((s) => s.toggle);
  const show = useOverlaysStore((s) => s.show);
  const persistTerminals = useTerminalsStore((s) => s.persist);
  const closeProject = useSessionStore((s) => s.closeProject);

  return (
    <div className="topbar">
      <button
        className={`sidebar-toggle ${leftOpen ? "open" : ""}`}
        onClick={toggleLeft}
        title={leftOpen ? "Hide workspace" : "Show workspace"}
      >
        <span className="sidebar-toggle-icon">
          <Icon name="panel-left" size={14} />
        </span>
        <span>Workspace</span>
        {changesCount > 0 && <span className="sidebar-toggle-badge">{changesCount}</span>}
      </button>

      <button
        className="tb-project"
        onClick={copyProjectPath}
        title={`${project.root}\n(click to copy path)`}
      >
        <span className="tb-project-name">{project.name}</span>
      </button>

      <span className="spacer" />

      <button
        className={`topbar-icon ${rightOpen ? "on" : ""}`}
        onClick={toggleRight}
        title={rightOpen ? "Hide side panel (Ctrl+J)" : "Show side panel (Ctrl+J)"}
      >
        <Icon name="panel-right" size={14} />
      </button>
      <button
        className="topbar-icon"
        onClick={toggleTheme}
        title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} size={14} />
      </button>
      <button
        className="topbar-icon"
        onClick={() => show("shortcuts")}
        title="Keyboard shortcuts (Ctrl+/)"
        aria-label="Keyboard shortcuts"
      >
        <Icon name="command" size={14} />
      </button>
      <button
        className="topbar-icon"
        onClick={() => show("gettingStarted")}
        title="Getting started guide"
        aria-label="Getting started guide"
      >
        <Icon name="help-circle" size={14} />
      </button>
      <button
        className="topbar-icon"
        onClick={() => show("about")}
        title="About Agent Console"
        aria-label="About Agent Console"
      >
        <Icon name="info" size={14} />
      </button>
      <button
        className="tb-close"
        onClick={() => {
          persistTerminals();
          closeProject();
        }}
        title="Close project"
      >
        Close
      </button>
    </div>
  );
}
