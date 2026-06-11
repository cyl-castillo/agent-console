import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc } from "../ipc/tauri";
import { useTerminalsStore } from "./terminalsStore";
import { useToastStore } from "./toastStore";
import type { VoiceModelProgress } from "../types/domain";
import type { TermInputDetail } from "../components/Terminal";

export type VoicePhase = "off" | "loading" | "ready" | "listening" | "transcribing";

interface VoiceState {
  phase: VoicePhase;
  /// Model download progress while phase === "loading" (null otherwise).
  progress: VoiceModelProgress | null;
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
  error: null,

  toggle: async () => {
    const { phase } = get();
    if (phase === "loading" || phase === "transcribing") return;
    if (phase === "off") {
      set({ phase: "loading", error: null, progress: null });
      try {
        await ipc.voiceEnable();
        set({ phase: "ready", progress: null });
      } catch (e) {
        set({ phase: "off", error: String(e), progress: null });
        useToastStore.getState().show(`Voice: ${String(e)}`, "error");
      }
    } else {
      try { await ipc.voiceDisable(); } catch { /* ignore */ }
      set({ phase: "off", progress: null });
    }
  },

  pttStart: async () => {
    if (get().phase !== "ready") return;
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
