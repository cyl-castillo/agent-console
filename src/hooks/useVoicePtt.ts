import { useEffect } from "react";

import { useVoiceStore } from "../stores/voiceStore";

/// Push-to-talk: hold Ctrl+Space to record, release Space to transcribe into
/// the active session's composer. Ctrl+Shift+V toggles voice mode on/off.
/// Capture-phase listeners so the hold wins over xterm's own key handling
/// (Ctrl+Space would otherwise send NUL to the PTY) — but only while voice
/// mode is on; with voice off the terminal sees every key as before.
export function useVoicePtt() {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code === "KeyV" && (e.ctrlKey || e.metaKey) && e.shiftKey) {
        e.preventDefault();
        void useVoiceStore.getState().toggle();
        return;
      }
      if (e.code === "Space" && (e.ctrlKey || e.metaKey) && !e.repeat) {
        const { phase, pttStart } = useVoiceStore.getState();
        if (phase === "ready") {
          e.preventDefault();
          e.stopPropagation();
          void pttStart();
        }
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.code !== "Space") return;
      const { phase, pttStop } = useVoiceStore.getState();
      if (phase === "listening") {
        e.preventDefault();
        e.stopPropagation();
        void pttStop();
      }
    };
    // Losing focus mid-hold means we'll never see the keyup — drop the take.
    const onBlur = () => {
      void useVoiceStore.getState().pttCancel();
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", onBlur);
    };
  }, []);
}
