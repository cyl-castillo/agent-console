import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { DEFAULT_AGENT, type AgentKind } from "../agents/profiles";
import { ipc } from "../ipc/tauri";
import type { AssistantLogSummary } from "../types/domain";
import { useModelStore } from "./modelStore";
import { useSessionStore } from "./sessionStore";

/// Mirror of `assistant_service::MAX_MESSAGE_CHARS`.
export const ASSISTANT_MAX_CHARS = 4000;

/// The human's opt-in to the learning log, per machine. Off until they say so.
const LEARN_KEY = "agent-console.assistant.learn.v1";

function loadRemember(): boolean {
  try {
    return localStorage.getItem(LEARN_KEY) === "1";
  } catch {
    return false;
  }
}

export type AssistantRating = "up" | "down";

export interface AssistantMessage {
  id: string;
  role: "user" | "assistant" | "error";
  text: string;
  atMs: number;
  /// The reply's id in the learning log; without it there is nothing to rate.
  logId?: string;
  rating?: AssistantRating;
}

/// What the turn in flight is doing, as one short line.
export function activityLine(kind: string, label: string): string | null {
  if (kind === "tool") {
    const tool = label.replace(/^mcp__agent_console__/, "").replace(/_/g, " ");
    return tool ? `Using ${tool}…` : "Looking at the console…";
  }
  if (kind === "thinking") return "Thinking…";
  if (kind === "text") return "Writing…";
  return null;
}

interface AssistantState {
  /// The project the conversation belongs to; a different one starts over.
  project: string | null;
  messages: AssistantMessage[];
  status: "idle" | "working";
  activity: string | null;
  /// The engine's session id of the last reply, to continue the conversation.
  sessionId: string | null;
  /// The engine the conversation runs on, fixed by its first turn.
  engine: AgentKind | null;
  /// False when the last reply ran without the console's tools.
  toolsAttached: boolean;
  tokens: number;

  /// Resolves to the reply's text, or null when the turn did not run or
  /// failed (the error is in `messages`).
  ask: (text: string, opts?: { spoken?: boolean }) => Promise<string | null>;
  reset: () => void;

  /// Learning log: the human's opt-in, what the log holds, and the last
  /// proposals drawn from it.
  remember: boolean;
  learning: AssistantLogSummary | null;
  proposing: boolean;
  learningError: string | null;
  setRemember: (on: boolean) => void;
  /// Rate a logged reply; the same rating again clears it.
  rate: (messageId: string, rating: AssistantRating) => Promise<void>;
  refreshLearning: () => Promise<void>;
  propose: () => Promise<void>;
  clearLog: () => Promise<void>;
  _onActivity: (kind: string, label: string) => void;
}

let seq = 0;
const nextId = () => `m${Date.now()}-${++seq}`;
/// Bumped on reset and project change: a reply that lands after either
/// belongs to a conversation that no longer exists and is dropped.
let generation = 0;

const empty = {
  messages: [] as AssistantMessage[],
  status: "idle" as const,
  activity: null,
  sessionId: null,
  engine: null,
  toolsAttached: true,
  tokens: 0,
};

export const useAssistantStore = create<AssistantState>((set, get) => ({
  project: null,
  ...empty,
  remember: loadRemember(),
  learning: null,
  proposing: false,
  learningError: null,

  ask: async (raw, opts) => {
    const text = raw.trim();
    if (!text || get().status === "working") return null;
    const root = useSessionStore.getState().project?.root ?? null;
    if (!root) return null;
    if (get().project !== root) {
      generation += 1;
      set({ project: root, ...empty });
    }
    const engine = get().engine ?? useModelStore.getState().defaultAgentFor(root) ?? DEFAULT_AGENT;
    const mine = generation;
    set((s) => ({
      engine,
      status: "working",
      activity: "Starting…",
      messages: [...s.messages, { id: nextId(), role: "user", text, atMs: Date.now() }],
    }));
    try {
      const reply = await ipc.assistantAsk(
        text,
        engine,
        get().sessionId,
        get().remember,
        opts?.spoken ?? false,
      );
      if (mine !== generation) return null;
      const answer = reply.text || "(the assistant returned no text)";
      set((s) => ({
        status: "idle",
        activity: null,
        sessionId: reply.sessionId ?? s.sessionId,
        toolsAttached: reply.toolsAttached,
        tokens: s.tokens + reply.tokens,
        messages: [
          ...s.messages,
          {
            id: nextId(),
            role: "assistant",
            text: answer,
            atMs: Date.now(),
            ...(reply.logId ? { logId: reply.logId } : {}),
          },
        ],
      }));
      return reply.text || null;
    } catch (err) {
      if (mine !== generation) return null;
      set((s) => ({
        status: "idle",
        activity: null,
        messages: [
          ...s.messages,
          {
            id: nextId(),
            role: "error",
            text: err instanceof Error ? err.message : String(err),
            atMs: Date.now(),
          },
        ],
      }));
      return null;
    }
  },

  reset: () => {
    generation += 1;
    set({ ...empty });
  },

  setRemember: (on) => {
    try {
      localStorage.setItem(LEARN_KEY, on ? "1" : "0");
    } catch {
      /* the choice still holds for this run */
    }
    set({ remember: on });
  },

  rate: async (messageId, rating) => {
    const msg = get().messages.find((m) => m.id === messageId);
    if (!msg?.logId) return;
    const before = msg.rating;
    const next = before === rating ? undefined : rating;
    const put = (r: AssistantRating | undefined) =>
      set((s) => ({
        messages: s.messages.map((m) => (m.id === messageId ? { ...m, rating: r } : m)),
      }));
    put(next);
    try {
      await ipc.assistantLogRate(msg.logId, next ?? null);
    } catch (err) {
      // What is on screen must be what is on disk.
      put(before);
      set({ learningError: err instanceof Error ? err.message : String(err) });
    }
  },

  refreshLearning: async () => {
    try {
      set({ learning: await ipc.assistantLogSummary(), learningError: null });
    } catch (err) {
      set({ learningError: err instanceof Error ? err.message : String(err) });
    }
  },

  propose: async () => {
    if (get().proposing) return;
    const root = useSessionStore.getState().project?.root ?? null;
    if (!root) return;
    const engine = get().engine ?? useModelStore.getState().defaultAgentFor(root) ?? DEFAULT_AGENT;
    set({ proposing: true, learningError: null });
    try {
      const lastProposals = await ipc.assistantPropose(engine);
      set((s) => ({
        proposing: false,
        learning: s.learning ? { ...s.learning, lastProposals } : s.learning,
      }));
      await get().refreshLearning();
    } catch (err) {
      set({ proposing: false, learningError: err instanceof Error ? err.message : String(err) });
    }
  },

  clearLog: async () => {
    try {
      await ipc.assistantLogClear();
      // Nothing on screen may point at a turn that no longer exists.
      set((s) => ({
        learningError: null,
        messages: s.messages.map(({ logId: _l, rating: _r, ...m }) => m),
      }));
      await get().refreshLearning();
    } catch (err) {
      set({ learningError: err instanceof Error ? err.message : String(err) });
    }
  },

  _onActivity: (kind, label) => {
    if (get().status !== "working") return;
    const line = activityLine(kind, label);
    if (line) set({ activity: line });
  },
}));

/// Mount once: the turn in flight reports what it is doing.
export async function attachAssistantListeners(): Promise<UnlistenFn> {
  return listen<{ kind: string; label: string }>("assistant://activity", (e) =>
    useAssistantStore.getState()._onActivity(e.payload.kind, e.payload.label),
  );
}
