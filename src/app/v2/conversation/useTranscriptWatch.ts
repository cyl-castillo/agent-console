import { useEffect } from "react";
import { ipc } from "../../../ipc/tauri";
import { itemsFor, useTranscriptStore, type TranscriptItem } from "../../../stores/transcriptStore";
import type { TerminalSession } from "../../../stores/terminalsStore";

/// Follow the CLI's transcript for the session on stage: the Rust side tails
/// the file while this is mounted and streams `transcript://delta`.
export function useTranscriptWatch(session: TerminalSession | null): readonly TranscriptItem[] {
  const sid = session?.agentSessionId;
  const engine = session?.agent ?? "claude";
  useEffect(() => {
    if (!sid) return;
    void ipc.transcriptWatch(sid, engine).catch(() => {});
    return () => {
      void ipc.transcriptUnwatch(sid).catch(() => {});
    };
  }, [sid, engine]);
  return useTranscriptStore(itemsFor(sid));
}
