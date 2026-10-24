import { create } from "zustand";
import type { ProgramStatus } from "../lib/programStatus";

/// F5: what each session's CLI says about itself through OSC 7501 — a
/// fact from the program, kept beside the hook-derived state. `blocked`
/// carries why (permission, question).
export interface SessionProgramStatus extends ProgramStatus {
  ts: number;
}

interface ProgramStatusState {
  bySession: Record<string, SessionProgramStatus>;
  note: (sessionId: string, status: ProgramStatus, ts?: number) => void;
  forget: (sessionId: string) => void;
}

export const useProgramStatusStore = create<ProgramStatusState>((set) => ({
  bySession: {},
  note: (sessionId, status, ts = Date.now()) =>
    set((s) => {
      if (status.state === "clear") {
        if (!(sessionId in s.bySession)) return s;
        const bySession = { ...s.bySession };
        delete bySession[sessionId];
        return { bySession };
      }
      return { bySession: { ...s.bySession, [sessionId]: { ...status, ts } } };
    }),
  forget: (sessionId) =>
    set((s) => {
      if (!(sessionId in s.bySession)) return s;
      const bySession = { ...s.bySession };
      delete bySession[sessionId];
      return { bySession };
    }),
}));
