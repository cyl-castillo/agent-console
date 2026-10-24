import { useEffect, useState } from "react";
import { usePolicyStore } from "../stores/policyStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { Chip } from "../ui/Chip";

/// F6: the project's approval policy as code — one JSON file in the repo
/// (`.agent-console/policy.json`), compiled into what Claude, Codex and
/// OpenCode each enforce. Edit, save, apply; the hash is what every
/// approval decided under it carries in the ledger.
export function PolicySection() {
  const status = usePolicyStore((s) => s.status);
  const error = usePolicyStore((s) => s.error);
  const loading = usePolicyStore((s) => s.loading);
  const save = usePolicyStore((s) => s.save);
  const apply = usePolicyStore((s) => s.apply);
  const load = usePolicyStore((s) => s.load);
  const root = useSessionStore((s) => s.project?.root);
  const showToast = useToastStore((s) => s.show);
  const [text, setText] = useState("");
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!editing && status) setText(JSON.stringify(status.policy, null, 2));
  }, [status, editing]);
  if (!root) return null;
  let parseError: string | null = null;
  try {
    JSON.parse(text);
  } catch (e) {
    parseError = String(e);
  }
  const c = status?.compiled;
  const counts = c
    ? {
        claude: c.claude.allow.length + c.claude.ask.length + c.claude.deny.length,
        codex: c.codexRules.length,
        opencode: Object.values(c.opencodePermission).reduce(
          (n, o) => n + Object.keys(o).length,
          0,
        ),
      }
    : null;
  const doSave = async () => {
    setBusy(true);
    try {
      await save(text);
      setEditing(false);
      showToast("Policy saved — apply it to compile for the three engines", "success");
    } catch (e) {
      showToast(`Policy not saved: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };
  const doApply = async () => {
    setBusy(true);
    try {
      const st = await apply();
      showToast(
        st
          ? `Policy ${st.hash.slice(0, 12)} applied to Claude, Codex and OpenCode — recorded as policy_set`
          : "Nothing applied",
        "success",
      );
    } catch (e) {
      showToast(`Policy not applied: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="wb-section" data-testid="policy-section">
      <div className="wb-section-title">
        approval policy as code
        {status && (
          <Chip mono tone={status.errors.length ? "danger" : status.exists ? "ok" : "neutral"}>
            {status.exists
              ? `${status.policy.rules.length} rules · ${status.hash.slice(0, 12)}`
              : "no policy file"}
          </Chip>
        )}
      </div>
      <p className="wb-hint">
        One file in the repo, <code>.agent-console/policy.json</code>, compiled into what each
        engine enforces: Claude <code>permissions</code>, Codex <code>prefix_rule</code> lines,
        OpenCode <code>permission</code>. Rules: <code>effect</code> allow · ask · block,{" "}
        <code>tool</code> shell · edit · read · fetch · mcp, <code>pattern</code> (a command prefix
        like <code>git *</code>, a path glob, a domain). Every approval decided while a policy is in
        force carries its hash.
        {loading && " · reading…"}
      </p>
      <textarea
        aria-label="Approval policy JSON"
        className="w-full rounded-2 border border-border bg-surface-2 p-2 font-code text-caption text-foreground"
        rows={10}
        value={text}
        spellCheck={false}
        onChange={(e) => {
          setEditing(true);
          setText(e.target.value);
        }}
      />
      {parseError && <p className="wb-hint text-danger">{parseError}</p>}
      {status && status.errors.length > 0 && (
        <ul className="m-0 list-none p-0 text-caption text-danger">
          {status.errors.map((e) => (
            <li key={e}>{e}</li>
          ))}
        </ul>
      )}
      {error && <p className="wb-hint text-danger">{error}</p>}
      {counts && (
        <p className="wb-hint">
          compiles to: Claude {counts.claude} rule{counts.claude === 1 ? "" : "s"} · Codex{" "}
          {counts.codex} prefix rule{counts.codex === 1 ? "" : "s"} · OpenCode {counts.opencode}{" "}
          pattern
          {counts.opencode === 1 ? "" : "s"}
          {c && c.skipped.length > 0 && (
            <>
              {" "}
              · {c.skipped.length} not expressible:{" "}
              <span className="text-faint-foreground">{c.skipped.join("; ")}</span>
            </>
          )}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <button
          className="wb-cta"
          disabled={busy || !!parseError || !editing}
          onClick={() => void doSave()}
        >
          save policy
        </button>
        <button
          className="wb-cta"
          disabled={busy || !status?.exists || (status?.errors.length ?? 0) > 0 || editing}
          onClick={() => void doApply()}
          title={
            editing
              ? "Save first"
              : "Write the compiled rules into the three engines' files and record policy_set"
          }
        >
          apply to Claude · Codex · OpenCode
        </button>
        <button
          className="wb-cta"
          disabled={busy}
          onClick={() => {
            setEditing(false);
            void load(root);
          }}
        >
          reload
        </button>
      </div>
    </section>
  );
}
