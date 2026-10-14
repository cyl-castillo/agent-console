/// Runs a spoken utterance: gathers the context from the stores, resolves it
/// with the grammar, performs the action, and acknowledges — a cue tone plus
/// a toast, so the user always knows what the console did with their words.

import { ipc } from "../ipc/tauri";
import { assessCommand } from "../permissions/rules";
import { readScreen } from "../lib/termScreen";
import { sendKeysToActiveSession, typeIntoActiveSession } from "../lib/termInput";
import { useApprovalStore } from "../stores/approvalStore";
import { availablePaletteActions, runPaletteAction } from "../stores/paletteStore";
import { useTerminalsStore } from "../stores/terminalsStore";
import { useToastStore } from "../stores/toastStore";
import { useUIStore } from "../stores/uiStore";
import { useVoiceStore } from "../stores/voiceStore";
import {
  analyzeScreen,
  helpGroups,
  helpSpeech,
  resolveVoice,
  type VoiceAction,
  type VoiceContext,
} from "./grammar";

export function voiceContext(): VoiceContext {
  const terms = useTerminalsStore.getState();
  const active = terms.activeId;
  return {
    lang: useVoiceStore.getState().lang,
    approvalPending: useApprovalStore.getState().queue.length > 0,
    screen: active ? analyzeScreen(readScreen(active)) : { kind: "none" },
    sessions: terms.sessions
      .filter((s) => !s.archived)
      .map((s) => ({ id: s.id, name: s.name, cwd: s.cwd })),
    activeSessionId: active,
    commands: availablePaletteActions(),
  };
}

function cue(kind: "ack" | "error" | "done"): void {
  void ipc.voiceEarcon(kind).catch(() => {});
}

function toast(msg: string, level: "info" | "success" | "error" = "info"): void {
  useToastStore.getState().show(msg, level);
}

/// Resolve and perform. Returns the action taken (for tests and the pill).
export async function runVoiceCommand(transcript: string): Promise<VoiceAction> {
  const ctx = voiceContext();
  const action = resolveVoice(transcript, ctx);
  await perform(action, ctx);
  return action;
}

async function perform(action: VoiceAction, ctx: VoiceContext): Promise<void> {
  const es = ctx.lang.startsWith("es");
  switch (action.kind) {
    case "approve":
    case "deny": {
      const req = useApprovalStore.getState().queue[0];
      if (!req) {
        cue("error");
        toast(es ? "No hay aprobación pendiente" : "No approval is pending");
        return;
      }
      // Same gate as the modal's Ctrl+Enter and the spoken flow: a dangerous
      // command needs a deliberate click, never a word.
      const cmd = typeof req.input?.command === "string" ? req.input.command : "";
      const dangerous =
        action.kind === "approve" &&
        req.tool === "Bash" &&
        assessCommand(cmd)?.level === "dangerous";
      if (dangerous) {
        cue("error");
        void useVoiceStore
          .getState()
          .say(
            es
              ? "Esa acción es peligrosa: confirmala en pantalla."
              : "That action is dangerous: confirm it on screen.",
          )
          .catch(() => {});
        return;
      }
      await useApprovalStore
        .getState()
        .decide(req.id, action.kind === "approve" ? "allow" : "deny", "by voice");
      cue("ack");
      return;
    }
    case "keys": {
      if (!sendKeysToActiveSession(action.data)) {
        cue("error");
        toast(es ? "No hay una sesión viva" : "No live session");
        return;
      }
      cue("ack");
      return;
    }
    case "session": {
      const terms = useTerminalsStore.getState();
      const s = terms.sessions.find((t) => t.id === action.id);
      if (!s) {
        cue("error");
        return;
      }
      if (s.status === "stopped") terms.resume(s.id);
      else terms.setActive(s.id);
      useUIStore.getState().setTab("terminal");
      cue("ack");
      toast(action.name);
      return;
    }
    case "palette": {
      const ran = await runPaletteAction(action.id);
      if (!ran) {
        cue("error");
        toast(es ? `No disponible ahora: ${action.label}` : `Not available now: ${action.label}`);
        return;
      }
      cue("ack");
      toast(action.label);
      return;
    }
    case "dictate": {
      const ok = await typeIntoActiveSession(action.text, { submit: action.submit });
      if (ok) cue(action.submit ? "done" : "ack");
      return;
    }
    case "help": {
      const groups = helpGroups(ctx.lang);
      toast(groups.map((g) => `${g.title}: ${g.phrases.join(" · ")}`).join("\n"), "info");
      void useVoiceStore
        .getState()
        .say(helpSpeech(ctx.lang))
        .catch(() => {});
      return;
    }
    case "silence": {
      await useVoiceStore.getState().hush();
      return;
    }
    case "unknown": {
      cue("error");
      const why =
        action.reason === "nothing-to-confirm"
          ? es
            ? "no hay nada que confirmar"
            : "nothing to confirm"
          : action.reason === "too-short"
            ? es
              ? "muy corto — decí «escribí» antes para dictar"
              : "too short — say “type” first to dictate"
            : es
              ? "sin comando — decí «qué puedo decir»"
              : "no command — say “what can I say”";
      toast(`${es ? "No entendí" : "Didn't catch that"}: «${action.heard}» (${why})`);
      return;
    }
  }
}
