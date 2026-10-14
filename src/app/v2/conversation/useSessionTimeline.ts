import { useEffect, useMemo } from "react";
import { buildTimeline, useProofStore, type TimelineTurn } from "../../../stores/proofStore";
import { useSessionStore } from "../../../stores/sessionStore";
import { useTerminalsStore, type TerminalSession } from "../../../stores/terminalsStore";

/// The active session's turns in chronological order, from the witnessed
/// ledger (the same events the Turns panel reads). Events bound to this
/// terminal, plus turn-bound events that carry no terminal of their own (a
/// `commit` is the human's act, recorded against the turn whose diff it
/// shipped).
export function useSessionTimeline(): { session: TerminalSession | null; turns: TimelineTurn[] } {
  const project = useSessionStore((s) => s.project);
  const sessions = useTerminalsStore((s) => s.sessions);
  const activeId = useTerminalsStore((s) => s.activeId);
  const events = useProofStore((s) => s.events);
  const proofRoot = useProofStore((s) => s.projectRoot);
  const load = useProofStore((s) => s.load);
  const session = sessions.find((s) => s.id === activeId) ?? null;

  useEffect(() => {
    if (project && !proofRoot) void load(project.root);
  }, [project, proofRoot, load]);

  const turns = useMemo(() => {
    if (!session) return [];
    const turnIds = new Set(
      events.filter((e) => e.termId === session.id && e.turnId).map((e) => e.turnId as string),
    );
    const mine = events.filter(
      (e) => e.termId === session.id || (!e.termId && !!e.turnId && turnIds.has(e.turnId)),
    );
    return buildTimeline(mine);
  }, [events, session]);

  return { session, turns };
}
