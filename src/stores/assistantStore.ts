import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { DEFAULT_AGENT, type AgentKind } from "../agents/profiles";
import { ipc } from "../ipc/tauri";
import { useModelStore } from "./modelStore";
import { useSessionStore } from "./sessionStore";

/// Mirror of `assistant_service::MAX_MESSAGE_CHARS`.
export const ASSISTANT_MAX_CHARS = 4000;

export interface AssistantMessage {
  id: string;
  role: "user" | "assistant" | "error";
  text: string;
  atMs: number;
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
  ask: (text: string) => Promise<string | null>;
  reset: () => void;
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

  ask: async (raw) => {
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
      const reply = await ipc.assistantAsk(text, engine, get().sessionId);
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
