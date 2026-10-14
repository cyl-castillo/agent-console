import { useEffect, useState } from "react";
import { Copy } from "lucide-react";
import { ipc } from "../ipc/tauri";
import type { ConsoleMcpConfig } from "../types/domain";
import { useToastStore } from "../stores/toastStore";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";
import { Field, Input } from "../ui/Field";

/// "This console as an MCP server": what to paste into Claude Code or Codex
/// so an agent can drive this console — with its own name, which the ledger
/// records as the actor of everything it does here.
export function ConsoleMcpSection() {
  const [caller, setCaller] = useState("claude");
  const [config, setConfig] = useState<ConsoleMcpConfig | null>(null);
  const [error, setError] = useState<string | null>(null);
  const showToast = useToastStore((s) => s.show);

  useEffect(() => {
    let cancelled = false;
    ipc
      .consoleMcpConfig(caller.trim() || undefined)
      .then((c) => {
        if (!cancelled) {
          setConfig(c);
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [caller]);

  const copy = (text: string, what: string) => {
    void navigator.clipboard?.writeText(text).then(
      () => showToast(`${what} copied`, "success"),
      () => showToast("Couldn't copy", "error"),
    );
  };

  return (
    <section className="wb-section" data-testid="console-mcp-section">
      <div className="wb-section-title">this console as an MCP server</div>
      <p className="wb-hint">
        Point an agent at this console and it can read the state, run the action contract, open
        sessions, send prompts, read the ledger and search memory — under its own name. Every call
        lands in the ledger as <code>agent:&lt;name&gt;</code>, never as you. Actions that kill,
        delete, push or spend are refused.
      </p>
      <div className="flex flex-col gap-2 py-2 font-sans text-body">
        <Field label="Agent name (what the ledger records)">
          <Input
            value={caller}
            onChange={(e) => setCaller(e.target.value)}
            placeholder="claude"
            className="max-w-xs"
            aria-label="Agent name"
          />
        </Field>
        {error && <p className="m-0 text-small text-danger">{error}</p>}
        {config && (
          <>
            <div className="flex flex-wrap gap-1">
              {config.tools.map((t) => (
                <Chip key={t} mono>
                  {t}
                </Chip>
              ))}
            </div>
            <div className="flex items-center gap-2">
              <span className="text-small text-muted-foreground">Claude Code</span>
              <Button
                size="sm"
                icon={<Copy size={14} />}
                onClick={() => copy(config.claudeAdd, "claude mcp add command")}
              >
                Copy command
              </Button>
            </div>
            <pre className="m-0 overflow-auto rounded-2 border border-border bg-background p-2 font-code text-small">
              {config.claudeAdd}
            </pre>
            <div className="flex items-center gap-2">
              <span className="text-small text-muted-foreground">Codex (~/.codex/config.toml)</span>
              <Button
                size="sm"
                icon={<Copy size={14} />}
                onClick={() => copy(config.codexToml, "TOML snippet")}
              >
                Copy snippet
              </Button>
            </div>
            <pre className="m-0 overflow-auto rounded-2 border border-border bg-background p-2 font-code text-small">
              {config.codexToml}
            </pre>
          </>
        )}
      </div>
    </section>
  );
}
