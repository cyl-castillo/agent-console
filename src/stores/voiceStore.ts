import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import { useApprovalStore } from "./approvalStore";
import { useTerminalsStore } from "./terminalsStore";
import { useToastStore } from "./toastStore";
import { assessCommand } from "../permissions/rules";
import type { ApprovalRequest, VoiceModelProgress } from "../types/domain";
import type { TermInputDetail } from "../components/Terminal";

export type VoicePhase = "off" | "loading" | "ready" | "listening" | "transcribing";

interface VoiceState {
  phase: VoicePhase;
  /// Model download progress while phase === "loading" (null otherwise).
  progress: VoiceModelProgress | null;
  /// Voice-input language reported by the backend ("es" by default).
  lang: string;
  /// Non-null while an approval is being announced / spoken-confirmed.
  approvalStage: "speaking" | "listening" | null;
  error: string | null;
  toggle: () => Promise<void>;
  pttStart: () => Promise<void>;
  pttStop: () => Promise<void>;
  /// Abort a hold without typing anything (e.g. window lost focus mid-hold).
  pttCancel: () => Promise<void>;
}

export const useVoiceStore = create<VoiceState>((set, get) => ({
  phase: "off",
  progress: null,
  lang: "es",
  approvalStage: null,
  error: null,

  toggle: async () => {
    const { phase } = get();
    if (phase === "loading" || phase === "transcribing") return;
    if (phase === "off") {
      set({ phase: "loading", error: null, progress: null });
      try {
        const status = await ipc.voiceEnable();
        set({ phase: "ready", progress: null, lang: status.language });
      } catch (e) {
        set({ phase: "off", error: String(e), progress: null });
        useToastStore.getState().show(`Voice: ${String(e)}`, "error");
      }
    } else {
      try { await ipc.voiceDisable(); } catch { /* ignore */ }
      set({ phase: "off", progress: null, approvalStage: null });
    }
  },

  pttStart: async () => {
    if (get().phase !== "ready" || get().approvalStage) return;
    set({ phase: "listening" });
    try {
      await ipc.voicePttStart();
    } catch (e) {
      set({ phase: "ready", error: String(e) });
      useToastStore.getState().show(`Voice: ${String(e)}`, "error");
    }
  },

  pttStop: async () => {
    if (get().phase !== "listening") return;
    set({ phase: "transcribing" });
    try {
      const text = (await ipc.voicePttStop()).trim();
      if (text) {
        const sessionId = useTerminalsStore.getState().activeId;
        if (sessionId) {
          // Same path the model pill / drag-and-drop use: the Terminal owning
          // this session writes the text into its PTY (the agent composer).
          const detail: TermInputDetail = { sessionId, data: `${text} ` };
          window.dispatchEvent(new CustomEvent("ac:term-input", { detail }));
        }
      }
    } catch (e) {
      useToastStore.getState().show(`Voice: ${String(e)}`, "error");
    } finally {
      set({ phase: "ready" });
    }
  },

  pttCancel: async () => {
    if (get().phase !== "listening") return;
    try { await ipc.voicePttStop(); } catch { /* ignore */ }
    set({ phase: "ready" });
  },
}));

export async function attachVoiceListeners(): Promise<UnlistenFn> {
  return await listen<VoiceModelProgress>("voice://model-progress", (e) => {
    useVoiceStore.setState({ progress: e.payload });
  });
}

// --- Spoken approvals ---------------------------------------------------------
// With voice mode on, an incoming approval request is announced aloud and a
// short mic window listens for "sí" / "no". Anything else (silence, ambiguity,
// voice off, dangerous commands) leaves the visual ApprovalModal in charge.

const handledApprovals = new Set<string>();

/// Watch the approval queue and voice-announce each new head request.
export function attachVoiceApprovalWatcher(): () => void {
  return useApprovalStore.subscribe((s) => {
    const head = s.queue[0];
    if (head) void announceApproval(head);
  });
}

async function announceApproval(req: ApprovalRequest): Promise<void> {
  const voice = useVoiceStore.getState();
  if (voice.phase !== "ready" || voice.approvalStage) return;
  if (handledApprovals.has(req.id)) return;
  if (handledApprovals.size > 500) handledApprovals.clear();
  handledApprovals.add(req.id);

  const es = voice.lang.startsWith("es");
  const stillPending = () =>
    useApprovalStore.getState().queue.some((r) => r.id === req.id);
  // Same rule as the modal's Ctrl+Enter: dangerous commands need a deliberate
  // click — a spoken "sí" must not be able to bypass that gate.
  const cmd = typeof req.input?.command === "string" ? req.input.command : "";
  const dangerous = req.tool === "Bash" && assessCommand(cmd)?.level === "dangerous";

  try {
    useVoiceStore.setState({ approvalStage: "speaking" });
    const announce = speechFor(req, es);
    if (dangerous) {
      await ipc.voiceSpeak(
        es
          ? `${announce}. Esta acción es peligrosa: confírmala en pantalla.`
          : `${announce}. This action is dangerous: confirm it on screen.`,
      );
      return;
    }
    await ipc.voiceSpeak(es ? `${announce}. ¿Apruebo?` : `${announce}. Approve?`);
    if (!stillPending()) return;
    useVoiceStore.setState({ approvalStage: "listening" });
    const heard = await ipc.voiceListen(4);
    if (!stillPending()) return;
    const verdict = parseYesNo(heard);
    if (verdict === "yes") {
      await useApprovalStore.getState().decide(req.id, "allow", "approved by voice");
      void ipc.voiceSpeak(es ? "Hecho." : "Done.").catch(() => {});
    } else if (verdict === "no") {
      await useApprovalStore.getState().decide(req.id, "deny", "denied by voice");
      void ipc.voiceSpeak(es ? "Denegado." : "Denied.").catch(() => {});
    } else {
      void ipc.voiceSpeak(
        es ? "No te entendí. Usa el modal." : "I didn't catch that. Use the modal.",
      ).catch(() => {});
    }
  } catch {
    // TTS or mic hiccup — the visual modal is still there as fallback.
  } finally {
    useVoiceStore.setState({ approvalStage: null });
    // A request queued while we were busy never re-fires the subscriber.
    const next = useApprovalStore.getState().queue[0];
    if (next && !handledApprovals.has(next.id)) void announceApproval(next);
  }
}

/// Short spoken description of what the agent wants to do.
function speechFor(req: ApprovalRequest, es: boolean): string {
  const inp = req.input ?? {};
  if (req.tool === "Bash") {
    const desc = typeof inp.description === "string" && inp.description ? inp.description : null;
    const cmd = typeof inp.command === "string" ? inp.command : "";
    const what = desc ?? (cmd.length > 120 ? `${cmd.slice(0, 120)}…` : cmd);
    return es ? `El agente quiere ejecutar: ${what}` : `The agent wants to run: ${what}`;
  }
  if (typeof inp.file_path === "string") {
    const file = inp.file_path.split("/").pop() ?? inp.file_path;
    const verbEs: Record<string, string> = {
      Write: "escribir", Edit: "editar", MultiEdit: "editar",
      StrReplace: "editar", Read: "leer", NotebookEdit: "editar",
    };
    return es
      ? `El agente quiere ${verbEs[req.tool] ?? `usar ${req.tool} en`} el archivo ${file}`
      : `The agent wants to ${req.tool.toLowerCase()} the file ${file}`;
  }
  return es
    ? `El agente quiere usar la herramienta ${req.tool}`
    : `The agent wants to use the ${req.tool} tool`;
}

/// Tolerant yes/no parser. Diacritics are stripped ("sí" → "si"); a negative
/// anywhere wins over a positive ("no, dale" must not approve).
function parseYesNo(transcript: string): "yes" | "no" | "unclear" {
  const norm = transcript.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "");
  const words = norm.split(/[^a-zñ]+/).filter(Boolean);
  const NO = new Set(["no", "nope", "cancela", "cancelar", "niega", "deny", "denegar", "para"]);
  const YES = new Set([
    "si", "dale", "ok", "okay", "aprueba", "apruebo", "aprobar",
    "hazlo", "adelante", "claro", "confirmo", "yes", "approve", "sure",
  ]);
  if (words.some((w) => NO.has(w))) return "no";
  if (words.some((w) => YES.has(w))) return "yes";
  return "unclear";
}
