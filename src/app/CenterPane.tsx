import type { Project } from "../types/domain";
import { ChangesView } from "../components/ChangesView";
import { Composer } from "../components/Composer";
import { OnboardingBanner } from "../components/OnboardingBanner";
import { Preview } from "../components/Preview";
import { Terminal } from "../components/Terminal";
import { WorktreeBar } from "../components/WorktreeBar";
import { useChangesStore } from "../stores/changesStore";
import { useOverlaysStore } from "../stores/overlaysStore";
import { useTerminalsStore, type TerminalSession } from "../stores/terminalsStore";
import { useUIStore } from "../stores/uiStore";
import { newSession } from "./actions";

interface Props {
  project: Project;
  liveTerminals: TerminalSession[];
  activeTerminal: TerminalSession | undefined;
  activeStopped: boolean;
}

/// The center column: Terminal / Changes / Preview tabs. Panes stay mounted
/// and are hidden with display:none so live PTYs survive tab switches.
export function CenterPane({ project, liveTerminals, activeTerminal, activeStopped }: Props) {
  const tab = useUIStore((s) => s.tab);
  const setTab = useUIStore((s) => s.setTab);
  const composerOpen = useUIStore((s) => s.composerOpen);
  const setComposerOpen = useUIStore((s) => s.setComposerOpen);
  const changesCount = useChangesStore((s) => s.status?.changes.length ?? 0);
  const activeTerminalId = useTerminalsStore((s) => s.activeId);
  const resumeTerminal = useTerminalsStore((s) => s.resume);
  const showOverlay = useOverlaysStore((s) => s.show);

  return (
    <main className="panel center">
      <div className="tabs">
        <button
          className={`tab ${tab === "terminal" ? "active" : ""}`}
          onClick={() => setTab("terminal")}
          title="Ctrl+1"
        >
          <span>Terminal</span>
          <span className="tab-shortcut">Ctrl 1</span>
        </button>
        <button
          className={`tab ${tab === "changes" ? "active" : ""}`}
          onClick={() => setTab("changes")}
          title="Ctrl+2"
        >
          <span>Changes</span>
          {changesCount > 0 && <span className="tab-badge">{changesCount}</span>}
          <span className="tab-shortcut">Ctrl 2</span>
        </button>
        <button
          className={`tab ${tab === "preview" ? "active" : ""}`}
          onClick={() => setTab("preview")}
          title="Ctrl+3"
        >
          <span>Preview</span>
          <span className="tab-shortcut">Ctrl 3</span>
        </button>
      </div>
      <div className="tab-pane" style={{ display: tab === "terminal" ? "flex" : "none" }}>
        {liveTerminals.length === 0 && !activeStopped ? (
          <div className="terminal-empty">
            <div>
              <div className="terminal-empty-title">No active sessions</div>
              <div className="terminal-empty-copy">
                Each new session starts in this project and launches its agent automatically.
              </div>
            </div>
            <button className="btn btn-primary" onClick={newSession}>
              + New session
            </button>
          </div>
        ) : (
          <>
            <WorktreeBar />
            {activeStopped && activeTerminal && (
              <div className="terminal-empty">
                <div>
                  <div className="terminal-empty-title">Session ended</div>
                  <div className="terminal-empty-copy">
                    “{activeTerminal.name}” exited. Resume relaunches its shell and agent with the
                    saved scrollback.
                  </div>
                </div>
                <button
                  className="btn btn-primary"
                  onClick={() => resumeTerminal(activeTerminal.id)}
                >
                  ▸ Resume session
                </button>
              </div>
            )}
            {liveTerminals.length > 0 && (
              // Stays mounted (hidden) while a stopped session is active so
              // the other live PTYs survive the detour through this state.
              <div
                className="terminals-stack"
                style={{ display: activeStopped ? "none" : undefined }}
              >
                {liveTerminals.map((s) => (
                  <Terminal
                    key={s.id}
                    session={s}
                    visible={tab === "terminal" && s.id === activeTerminalId}
                  />
                ))}
              </div>
            )}
            {composerOpen && (
              <Composer projectRoot={project.root} onClose={() => setComposerOpen(false)} />
            )}
          </>
        )}
      </div>
      <div className="tab-pane" style={{ display: tab === "changes" ? "flex" : "none" }}>
        <ChangesView />
      </div>
      <div className="tab-pane" style={{ display: tab === "preview" ? "flex" : "none" }}>
        <Preview />
      </div>
      <OnboardingBanner onOpen={() => showOverlay("gettingStarted")} />
    </main>
  );
}
