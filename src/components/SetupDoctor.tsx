import { useEffect } from "react";

import { usePreflightStore } from "../stores/preflightStore";
import { startLoginSession } from "../lib/loginSession";
import { startInstallSession } from "../lib/installSession";
import { useToastStore } from "../stores/toastStore";
import type { AgentKind } from "../agents/profiles";
import type { EngineAuth, PreflightTool } from "../types/domain";

/// Native clipboard first (WebKitGTK reports success on navigator.clipboard
/// writes it silently drops), web API as a fallback elsewhere.
async function copyText(text: string): Promise<void> {
  try {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
  } catch {
    await navigator.clipboard.writeText(text);
  }
}

/// What each tool is for, in words a first-time user can act on.
const TOOL_ROLES: Record<string, string> = {
  claude: "the Claude Code agent",
  codex: "the Codex agent (optional second engine)",
  node: "runs the approval & memory bridges",
  git: "snapshots, diffs and proof",
};

function ToolRow({ tool }: { tool: PreflightTool }) {
  const toast = useToastStore((s) => s.show);
  return (
    <li className={`doctor-row ${tool.found ? "ok" : "miss"}`}>
      <span className="doctor-dot" aria-hidden>
        {tool.found ? "●" : "○"}
      </span>
      <div className="doctor-body">
        <div className="doctor-name">
          <code>{tool.name}</code>
          <span className="doctor-role">{TOOL_ROLES[tool.name] ?? ""}</span>
        </div>
        {tool.found ? (
          <div className="doctor-detail">{tool.version ?? "installed"}</div>
        ) : (
          <>
            <div className="doctor-detail">not found</div>
            {tool.fixCommand && (
              <div className="doctor-fix">
                <code className="doctor-cmd">{tool.fixCommand}</code>
                <button
                  className="wb-cta wb-cta-sm"
                  onClick={() => void startInstallSession(tool.name, tool.fixCommand as string)}
                >
                  Install
                </button>
                <button
                  className="wb-link"
                  onClick={() =>
                    void copyText(tool.fixCommand as string).then(() =>
                      toast("Command copied", "info"),
                    )
                  }
                >
                  copy
                </button>
              </div>
            )}
            {tool.fixNote && <div className="doctor-note">{tool.fixNote}</div>}
          </>
        )}
      </div>
    </li>
  );
}

function AuthRow({ auth }: { auth: EngineAuth }) {
  const state = auth.loggedIn === null ? "unknown" : auth.loggedIn ? "ok" : "miss";
  return (
    <li className={`doctor-row ${state}`}>
      <span className="doctor-dot" aria-hidden>
        {state === "ok" ? "●" : state === "miss" ? "○" : "◌"}
      </span>
      <div className="doctor-body">
        <div className="doctor-name">
          <code>{auth.engine}</code>
          <span className="doctor-role">login</span>
        </div>
        <div className="doctor-detail">
          {state === "ok"
            ? (auth.detail ?? "logged in")
            : state === "miss"
              ? "not logged in"
              : "state unknown"}
        </div>
      </div>
      {state === "miss" && (
        <button
          className="wb-cta wb-cta-sm"
          onClick={() => startLoginSession(auth.engine as AgentKind)}
        >
          Fix login
        </button>
      )}
    </li>
  );
}

/// The environment doctor: what the console needs, what's present, and the
/// official fix for anything missing. Fix commands are never run silently —
/// they are surfaced for the user to copy (and, since F2, typed review-first
/// into a visible terminal).
export function SetupDoctor() {
  const result = usePreflightStore((s) => s.result);
  const checking = usePreflightStore((s) => s.checking);
  const check = usePreflightStore((s) => s.check);

  useEffect(() => {
    void check();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!result) {
    return <div className="doctor-loading">{checking ? "Checking your setup…" : ""}</div>;
  }

  return (
    <div className="doctor">
      <ul className="doctor-list">
        {result.tools.map((t) => (
          <ToolRow key={t.name} tool={t} />
        ))}
        {result.auth.map((a) => (
          <AuthRow key={a.engine} auth={a} />
        ))}
      </ul>
      <button className="wb-link doctor-recheck" disabled={checking} onClick={() => void check()}>
        {checking ? "checking…" : "↻ re-check"}
      </button>
    </div>
  );
}
