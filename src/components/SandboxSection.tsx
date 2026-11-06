import { useEffect, useState } from "react";
import { ipc } from "../ipc/tauri";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { Chip } from "../ui/Chip";
import type { SandboxStatus } from "../types/domain";

/// SEGURIDAD-MOVIL F2: the one control that makes a forged approval
/// impossible. The agent's shell runs as you; with the engine's sandbox off
/// it could write the console's answer files itself — the console detects
/// that, denies it and interrupts the session, but only the sandbox
/// prevents it. One click turns it on for this project.
export function SandboxSection() {
  const project = useSessionStore((s) => s.project);
  const showToast = useToastStore((s) => s.show);
  const [status, setStatus] = useState<SandboxStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const root = project?.root ?? null;

  useEffect(() => {
    if (!root) return;
    let alive = true;
    void ipc
      .sandboxStatus(root)
      .then((s) => {
        if (alive) setStatus(s);
      })
      .catch(() => {
        if (alive) setStatus(null);
      });
    return () => {
      alive = false;
    };
  }, [root]);

  if (!root) return null;
  const on = status?.enabled === true;
  const tone = on ? "ok" : status?.enabled === false ? "danger" : "warn";
  const label = on
    ? `on · ${status?.source ?? "settings"}`
    : status?.enabled === false
      ? `off · ${status?.source ?? "settings"}`
      : "not set";

  return (
    <section className="wb-section" data-testid="sandbox-section">
      <div className="wb-section-title">
        agent sandbox
        <Chip tone={tone} mono data-testid="sandbox-state">
          {label}
        </Chip>
      </div>
      <p className="wb-hint">
        With the sandbox on, the agent's shell cannot write outside this project nor reach the
        console's port — so nothing it runs can answer its own approvals. Off, the console still
        detects a forged answer, denies it and interrupts the session, but detection is a net, not a
        wall. Hooks keep working either way.
      </p>
      {!on && (
        <div className="flex flex-wrap gap-2">
          <button
            className="wb-cta"
            disabled={busy}
            data-testid="sandbox-enable"
            onClick={() => {
              setBusy(true);
              void ipc
                .sandboxEnable(root)
                .then((s) => {
                  setStatus(s);
                  showToast(
                    s.enabled
                      ? "Sandbox on for this project — restart the agent's session to apply."
                      : "Written, but a more specific settings file still turns it off.",
                    s.enabled ? "info" : "error",
                  );
                })
                .catch((e) => showToast(`Couldn't enable the sandbox: ${String(e)}`, "error"))
                .finally(() => setBusy(false));
            }}
          >
            turn the sandbox on for this project
          </button>
        </div>
      )}
      <p className="wb-hint">
        Writes to{" "}
        <span className="font-code">{status?.projectSettings ?? ".claude/settings.json"}</span> (
        <span className="font-code">sandbox.enabled</span>). Claude Code only; Codex sessions use
        its <span className="font-code">workspace-write</span> sandbox.
      </p>
    </section>
  );
}
