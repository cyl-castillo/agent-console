import { useCallback, useEffect, useState } from "react";
import { Copy, RefreshCw } from "lucide-react";
import { ipc } from "../ipc/tauri";
import { copyText } from "../lib/clipboard";
import { useToastStore } from "../stores/toastStore";
import type { FirewallStatus } from "../types/domain";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";

const TONE: Record<string, "ok" | "warn" | "danger" | "neutral"> = {
  enforced: "ok",
  off: "neutral",
  drifted: "warn",
  unsafe: "danger",
  unsupported: "neutral",
};

const LABEL: Record<string, string> = {
  enforced: "on",
  off: "off",
  drifted: "out of date",
  unsafe: "not safe",
  unsupported: "not available",
};

const preClass =
  "m-0 max-h-64 overflow-auto rounded-2 border border-border bg-background p-2 font-code text-small";

/// Firewall mode: the console's hooks and a baseline policy in Claude Code's
/// managed settings, where the agent's own shell cannot edit them. The
/// console shows what would be installed and the commands; the user runs
/// them, with their administrator password, in their own terminal. Nothing
/// here elevates.
export function FirewallSection() {
  const showToast = useToastStore((s) => s.show);
  const [status, setStatus] = useState<FirewallStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(
    (announce: boolean) => {
      setBusy(true);
      return ipc
        .firewallStatus()
        .then((s) => {
          setStatus(s);
          setError(null);
          if (announce) showToast(`Firewall mode: ${LABEL[s.state] ?? s.state}`, "info");
        })
        .catch((e) => setError(String(e)))
        .finally(() => setBusy(false));
    },
    [showToast],
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  const copy = (text: string, what: string) => {
    void copyText(text).then(
      () => showToast(`${what} copied`, "success"),
      () => showToast("Couldn't copy", "error"),
    );
  };

  const state = status?.state ?? "off";
  const installLabel =
    state === "off" ? "Copy install commands" : "Copy commands to reinstall this version";

  return (
    <section className="wb-section" data-testid="firewall-section">
      <div className="wb-section-title">
        firewall mode
        {status && (
          <Chip tone={TONE[state] ?? "neutral"} mono data-testid="firewall-state">
            {LABEL[state] ?? state}
          </Chip>
        )}
      </div>
      <p className="wb-hint">
        Everything the console enforces lives in files the agent's own shell can write. Firewall
        mode moves the hooks and a baseline policy to Claude Code's managed settings, owned by the
        administrator of this machine: the agent cannot edit them, remove them, or switch them off.
        A hook that fails denies instead of letting the action through.
      </p>
      {error && <p className="m-0 font-sans text-small text-danger">{error}</p>}
      {status && status.problems.length > 0 && (
        <ul className="m-0 flex list-disc flex-col gap-1 pl-5 font-sans text-small text-foreground">
          {status.problems.map((p) => (
            <li key={p} data-testid="firewall-problem">
              {p}
            </li>
          ))}
        </ul>
      )}
      {status && state !== "unsupported" && (
        <div className="flex flex-col gap-2 py-2 font-sans text-body">
          <p className="m-0 text-small text-foreground" data-testid="firewall-consequences">
            While it is on, on every Claude Code session of this machine: only Agent Console's hooks
            run (your own hooks in <span className="font-code">~/.claude/settings.json</span> are
            ignored); allow rules in your settings stop applying, so "always allow" asks again;
            bypass mode is refused; the sandbox is mandatory and Claude Code will not start without
            it.
          </p>
          <ul className="m-0 flex list-none flex-col gap-1 p-0 text-small">
            {status.engines.map((e) => (
              <li
                key={e.engine}
                className="flex items-baseline gap-2"
                data-testid={`firewall-engine-${e.engine}`}
              >
                <Chip tone={e.covered ? "ok" : "neutral"} mono>
                  {e.engine}
                </Chip>
                <span className="text-muted-foreground">{e.note}</span>
              </li>
            ))}
          </ul>
          {status.installScript ? (
            <>
              <p className="m-0 text-small text-muted-foreground">
                Read the commands, then run them in your own terminal. They ask for your
                administrator password. Agent Console never runs them for you.
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  icon={<Copy size={14} />}
                  data-testid="firewall-copy-install"
                  onClick={() => copy(status.installScript ?? "", "Install commands")}
                >
                  {installLabel}
                </Button>
                <Button
                  size="sm"
                  icon={<RefreshCw size={14} />}
                  disabled={busy}
                  data-testid="firewall-check"
                  onClick={() => void load(true)}
                >
                  Check again
                </Button>
              </div>
              <pre className={preClass} data-testid="firewall-install-script" tabIndex={0}>
                {status.installScript}
              </pre>
            </>
          ) : (
            <>
              {state !== "enforced" && (
                <p
                  className="m-0 text-small text-muted-foreground"
                  data-testid="firewall-no-bridge"
                >
                  This build has no bridge binary to install. Install Agent Console from a release
                  to turn firewall mode on.
                </p>
              )}
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  icon={<RefreshCw size={14} />}
                  disabled={busy}
                  data-testid="firewall-check"
                  onClick={() => void load(true)}
                >
                  Check again
                </Button>
              </div>
            </>
          )}
          {status.uninstallScript && (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-small text-muted-foreground">To turn it off</span>
                <Button
                  size="sm"
                  icon={<Copy size={14} />}
                  data-testid="firewall-copy-uninstall"
                  onClick={() => copy(status.uninstallScript ?? "", "Removal commands")}
                >
                  Copy removal commands
                </Button>
              </div>
              <pre className={preClass} data-testid="firewall-uninstall-script" tabIndex={0}>
                {status.uninstallScript}
              </pre>
            </>
          )}
          <p className="wb-hint">
            Installs <span className="font-code">{status.dropinPath}</span> and{" "}
            <span className="font-code">{status.bridgePath}</span>. Restart the agent's sessions
            after installing or removing; <span className="font-code">/status</span> in Claude Code
            lists the managed settings it loaded.
          </p>
        </div>
      )}
    </section>
  );
}
