import { useEffect, useMemo, useState } from "react";
import { ipc } from "../../ipc/tauri";
import {
  handoffPayload,
  handoffPrompt,
  suggestOpencodeModel,
  type HandoffReason,
} from "../../lib/handoff";
import { useOpencodeModelsStore } from "../../stores/opencodeModelsStore";
import type { TimelineTurn } from "../../stores/proofStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useShellStore } from "../../stores/shellStore";
import { useSkillsStore } from "../../stores/skillsStore";
import { useTerminalsStore, type TerminalSession } from "../../stores/terminalsStore";
import { useToastStore } from "../../stores/toastStore";
import { useUIStore } from "../../stores/uiStore";

/// F10: "continue in OpenCode" — the hand-over a budget or quota card
/// offers. Picks a model from OpenCode's live catalog (open-weight first),
/// opens a new OpenCode session in the same folder seeded with the
/// hand-over prompt (typed, not sent: the human reads it), and witnesses
/// `model_switch {reason}` on the session that stopped. `run()` resolves
/// true once the new session exists — the cards record their own decision
/// on that; the ledger line is best-effort, like every witness.
export function useHandoff(
  session: TerminalSession,
  turn: TimelineTurn | undefined,
  reason: HandoffReason,
) {
  const root = useSessionStore((s) => s.project?.root);
  const models = useOpencodeModelsStore((s) => s.models);
  const loading = useOpencodeModelsStore((s) => s.loading);
  const load = useOpencodeModelsStore((s) => s.load);
  const available = useSkillsStore((s) => s.hooks?.opencodeAvailable ?? true);
  const add = useTerminalsStore((s) => s.add);
  const persist = useTerminalsStore((s) => s.persist);
  const setActive = useTerminalsStore((s) => s.setActive);
  const goTo = useShellStore((s) => s.goTo);
  const setCenterTab = useUIStore((s) => s.setTab);
  const showToast = useToastStore((s) => s.show);
  useEffect(() => {
    if (root && available) void load(root);
  }, [root, available, load]);
  const suggested = useMemo(() => suggestOpencodeModel(models), [models]);
  const [model, setModel] = useState<string>("");
  const chosen = model || suggested?.id || "";
  const [busy, setBusy] = useState(false);

  const run = async (): Promise<boolean> => {
    if (!root || !chosen) return false;
    setBusy(true);
    try {
      const ctx = {
        turn,
        fromEngine: session.agent ?? "claude",
        fromModel: session.model,
        reason,
      };
      const toTermId = add(
        session.cwd,
        `${session.name} ↪ opencode`,
        chosen,
        "opencode",
        session.worktree,
        undefined,
        handoffPrompt(ctx),
      );
      persist();
      // The session is the act; the ledger line is best-effort (witnessing
      // can be off for the project) — a failure must not invite a second
      // session.
      try {
        await ipc.testigoHandoff(
          root,
          session.id,
          session.agentSessionId,
          reason,
          session.model,
          chosen,
          "opencode",
          toTermId,
          handoffPayload(ctx),
        );
      } catch (e) {
        showToast(`Hand-over not witnessed: ${String(e)}`, "info");
      }
      setActive(toTermId);
      goTo("sessions");
      setCenterTab("terminal");
      showToast(`Continuing in OpenCode (${chosen}) — review the hand-over, then send`, "success");
      return true;
    } catch (e) {
      showToast(`Couldn't hand over: ${String(e)}`, "error");
      return false;
    } finally {
      setBusy(false);
    }
  };
  return { available, models, loading, chosen, setModel, busy, run };
}
