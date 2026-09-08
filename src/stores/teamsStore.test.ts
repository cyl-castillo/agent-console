import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TeamsChat, TeamsLoginPoll, TeamsMessage } from "../types/domain";

const world = vi.hoisted(() => ({
  polls: [] as TeamsLoginPoll[],
  pollCalls: 0,
  cancelCalls: 0,
  disconnectCalls: 0,
  chats: [] as TeamsChat[],
  chatsError: null as string | null,
  messagesByChat: {} as Record<string, TeamsMessage[]>,
  beginError: null as string | null,
  configured: false,
}));

vi.mock("../ipc/tauri", () => ({
  ipc: {
    teamsStatus: async () => ({
      configured: world.configured,
      clientId: "c",
      tenant: "organizations",
      account: "Carlos",
    }),
    teamsBeginLogin: async () => {
      if (world.beginError) throw new Error(world.beginError);
      return {
        userCode: "ABC-123",
        verificationUri: "https://microsoft.com/devicelogin",
        message: "go there",
        intervalMs: 10,
        expiresInSecs: 900,
      };
    },
    teamsPollLogin: async () => {
      world.pollCalls++;
      return world.polls.shift() ?? { state: "pending" };
    },
    teamsCancelLogin: async () => void world.cancelCalls++,
    teamsDisconnect: async () => void world.disconnectCalls++,
    teamsListChats: async () => {
      if (world.chatsError) throw new Error(world.chatsError);
      return world.chats;
    },
    teamsListMessages: async (chatId: string) => world.messagesByChat[chatId] ?? [],
  },
}));

import { useTeamsStore } from "./teamsStore";

/// The poll loop sleeps on real setTimeout(…, 10); flush by waiting it out.
const settle = () => new Promise((r) => setTimeout(r, 60));

beforeEach(() => {
  world.polls = [];
  world.pollCalls = 0;
  world.cancelCalls = 0;
  world.disconnectCalls = 0;
  world.chats = [];
  world.chatsError = null;
  world.messagesByChat = {};
  world.beginError = null;
  world.configured = false;
  useTeamsStore.setState({
    status: null,
    login: null,
    connecting: false,
    connectError: null,
    chats: [],
    chatsError: null,
    activeChatId: null,
    messages: [],
    messagesError: null,
  });
});

afterEach(async () => {
  vi.useRealTimers();
  // Orphan any loop a test left running before the next one starts.
  await useTeamsStore.getState().cancelLogin();
});

/// The poll loop clamps its interval to ≥1s; fake timers drive one tick.
async function tickPoll() {
  await vi.advanceTimersByTimeAsync(1000);
  await vi.advanceTimersByTimeAsync(0); // flush the poll's follow-up microtasks
}

describe("teams store", () => {
  it("login flow: begin shows the code, polls until connected, then loads status+chats", async () => {
    world.polls = [{ state: "pending" }, { state: "connected", account: "Carlos" }];
    world.configured = true;
    world.chats = [
      {
        id: "19:x@thread.v2",
        title: "Ana",
        chatType: "oneOnOne",
        lastPreview: "hola",
        lastActivity: "2026-09-08T13:00:00Z",
      },
    ];
    vi.useFakeTimers();
    await useTeamsStore.getState().beginLogin("c", "");
    expect(useTeamsStore.getState().login?.userCode).toBe("ABC-123");
    expect(useTeamsStore.getState().connecting).toBe(true);
    await tickPoll(); // pending
    await tickPoll(); // connected
    const s = useTeamsStore.getState();
    expect(s.connecting).toBe(false);
    expect(s.login).toBeNull();
    expect(s.status?.configured).toBe(true);
    expect(s.chats).toHaveLength(1);
    expect(world.pollCalls).toBe(2);
  });

  it("login failure surfaces the backend's message and clears the code screen", async () => {
    world.polls = [{ state: "failed", message: "admin approval required" }];
    vi.useFakeTimers();
    await useTeamsStore.getState().beginLogin("c", "");
    await tickPoll();
    const s = useTeamsStore.getState();
    expect(s.connectError).toBe("admin approval required");
    expect(s.login).toBeNull();
    expect(s.connecting).toBe(false);
  });

  it("cancelLogin orphans the poll loop and tells the backend", async () => {
    await useTeamsStore.getState().beginLogin("c", "");
    await useTeamsStore.getState().cancelLogin();
    const callsAtCancel = world.pollCalls;
    await settle();
    // The orphaned loop may have had one tick in flight; it must not keep going.
    expect(world.pollCalls).toBeLessThanOrEqual(callsAtCancel + 1);
    expect(world.cancelCalls).toBe(1);
    expect(useTeamsStore.getState().login).toBeNull();
  });

  it("beginLogin failure reports without starting a loop", async () => {
    world.beginError = "client ID must be the app registration's GUID";
    await useTeamsStore.getState().beginLogin("bad", "");
    await settle();
    const s = useTeamsStore.getState();
    expect(s.connectError).toContain("GUID");
    expect(s.connecting).toBe(false);
    expect(world.pollCalls).toBe(0);
  });

  it("openChat loads messages and ignores late responses for a left chat", async () => {
    world.messagesByChat = {
      a: [{ id: "1", from: "Ana", created: "2026-09-08T13:00:00Z", body: "hola" }],
      b: [{ id: "2", from: "Luis", created: "2026-09-08T14:00:00Z", body: "chau" }],
    };
    await useTeamsStore.getState().openChat("a");
    expect(useTeamsStore.getState().messages[0].body).toBe("hola");
    // Late response guard: switch chats, then resolve the old one.
    const old = useTeamsStore.getState().openChat("a");
    await useTeamsStore.getState().openChat("b");
    await old;
    expect(useTeamsStore.getState().activeChatId).toBe("b");
    expect(useTeamsStore.getState().messages[0].body).toBe("chau");
    useTeamsStore.getState().closeChat();
    expect(useTeamsStore.getState().activeChatId).toBeNull();
    expect(useTeamsStore.getState().messages).toEqual([]);
  });

  it("disconnect clears everything and stops a pending login", async () => {
    await useTeamsStore.getState().beginLogin("c", "");
    await useTeamsStore.getState().disconnect();
    await settle();
    const s = useTeamsStore.getState();
    expect(world.disconnectCalls).toBe(1);
    expect(s.status?.configured).toBe(false);
    expect(s.login).toBeNull();
    expect(s.chats).toEqual([]);
  });

  it("chat list errors are surfaced, not thrown", async () => {
    world.chatsError = "Microsoft Graph returned 429";
    await useTeamsStore.getState().refreshChats();
    expect(useTeamsStore.getState().chatsError).toContain("429");
    expect(useTeamsStore.getState().loadingChats).toBe(false);
  });
});
