import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  assessHooksHealth,
  EMPTY_SIGNAL,
  noteEvent,
  noteInput,
  suspiciousTerms,
  type HooksVerdict,
  type TermSignal,
} from "../lib/hooksHealth";
import { ipc } from "../ipc/tauri";
import { useSkillsStore } from "./skillsStore";
import { useTerminalsStore } from "./terminalsStore";
import { useToastStore } from "./toastStore";

/// The stateful half of lib/hooksHealth: collects the two signals and runs
/// the verdict on a calm cadence. See that module for why the verdict never
/// depends on hooks themselves.

/// `claude agents --json` is one process spawn; never more often than this,
/// and only when some terminal looks suspicious.
export const PROBE_MIN_INTERVAL_MS = 60_000;
/// How often the status bar re-evaluates.
export const CHECK_INTERVAL_MS = 15_000;

interface HooksHealthState {
  lastEventMs: number | null;
  perTerm: Record<string, TermSignal>;
  liveClaudeTerms: string[];
  lastProbeMs: number;
  /// Terminals already warned about with a toast — one warning per terminal.
  warned: string[];
  verdict: HooksVerdict;
  /// Terminal input (from the PTY, not from hooks).
  noteInput: (termId: string, data: string) => void;
  /// Any hook event; `termId` when the payload carries one.
  noteEvent: (termId?: string | null) => void;
  /// Re-evaluate; probes the CLI (rate-limited) only when needed.
  check: () => Promise<void>;
}

export const useHooksHealthStore = create<HooksHealthState>((set, get) => ({
  lastEventMs: null,
  perTerm: {},
  liveClaudeTerms: [],
  lastProbeMs: 0,
  warned: [],
  verdict: { kind: "silent" },

  noteInput: (termId, data) => {
    const now = Date.now();
    const prev = get().perTerm[termId] ?? EMPTY_SIGNAL;
    const next = noteInput(prev, data, now);
    if (next === prev) return;
    set((s) => ({ perTerm: { ...s.perTerm, [termId]: next } }));
  },

  noteEvent: (termId) => {
    const now = Date.now();
    set((s) => {
      const perTerm = termId
        ? { ...s.perTerm, [termId]: noteEvent(s.perTerm[termId] ?? EMPTY_SIGNAL, now) }
        : s.perTerm;
      return { lastEventMs: now, perTerm };
    });
  },

  check: async () => {
    const now = Date.now();
    // Forget terminals that are gone: a stopped session can't be "not reporting".
    const live = new Set(
      useTerminalsStore
        .getState()
        .sessions.filter((t) => t.status === "live")
        .map((t) => t.id),
    );
    const perTerm = Object.fromEntries(
      Object.entries(get().perTerm).filter(([id]) => live.has(id)),
    );
    let liveClaudeTerms = get().liveClaudeTerms.filter((t) => live.has(t));
    const suspicious = suspiciousTerms(perTerm, now).filter((t) => !liveClaudeTerms.includes(t));
    if (suspicious.length > 0 && now - get().lastProbeMs >= PROBE_MIN_INTERVAL_MS) {
      set({ lastProbeMs: now });
      try {
        const bindings = await ipc.termAgentSessions();
        liveClaudeTerms = bindings.map((b) => b.termKey).filter((t) => live.has(t));
      } catch {
        // No CLI answer ⇒ no proof of a live Claude ⇒ no alarm. Never guess.
      }
    }
    const hooks = useSkillsStore.getState().hooks;
    const verdict = assessHooksHealth({
      // Unknown status (not fetched yet) must not read as "off".
      installed: hooks?.installed !== false,
      lastEventMs: get().lastEventMs,
      perTerm,
      liveClaudeTerms: new Set(liveClaudeTerms),
      now,
    });
    set({ perTerm, liveClaudeTerms, verdict });
    if (verdict.kind === "stale") {
      const fresh = verdict.termIds.filter((t) => !get().warned.includes(t));
      if (fresh.length > 0) {
        set((s) => ({ warned: [...s.warned, ...fresh] }));
        const names = fresh
          .map((t) => useTerminalsStore.getState().sessions.find((x) => x.id === t)?.name ?? t)
          .join(", ");
        useToastStore
          .getState()
          .show(
            `Hooks aren't reporting for ${names}: prompts were sent but no hook event arrived, so approvals, proof, snapshots and resume are blind there. Likely the folder isn't trusted by Claude Code (hooks silently skip untrusted directories) — run \`claude\` there once and accept the trust prompt, or reinstall hooks from the status bar.`,
            "error",
          );
      }
    }
  },
}));

/// Every hook-borne event counts as "the bridge is alive", including approval
/// requests (PreToolUse) and tool results — not only the prompt hook.
export async function attachHooksHealthListeners(): Promise<UnlistenFn> {
  const note = (termId?: string | null) => useHooksHealthStore.getState().noteEvent(termId);
  const kinds = [
    "hook://user_prompt",
    "hook://tool_result",
    "hook://turn_end",
    "hook://turn_failed",
    "hook://model_switch",
    "hook://notification",
    "hook://approval_deferred",
    "hook://status",
    "approval://request",
  ];
  const offs: UnlistenFn[] = [];
  for (const k of kinds) {
    offs.push(await listen<{ termId?: string | null }>(k, (e) => note(e.payload?.termId)));
  }
  return () => {
    for (const off of offs) off();
  };
}
