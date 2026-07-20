import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ApprovalRequest } from "../types/domain";

// The store touches the backend and sibling stores at import time — isolate.
vi.mock("../ipc/tauri", () => ({
  ipc: {
    voiceEnable: vi.fn(),
    voiceDisable: vi.fn(),
    voicePttStart: vi.fn(),
    voicePttStop: vi.fn(),
    voiceSpeak: vi.fn(),
    voiceListen: vi.fn(),
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("./approvalStore", () => ({
  useApprovalStore: {
    subscribe: vi.fn(() => () => {}),
    getState: vi.fn(() => ({ queue: [], decide: vi.fn() })),
  },
}));
vi.mock("./terminalsStore", () => ({
  useTerminalsStore: { getState: vi.fn(() => ({ activeId: "term-1" })) },
}));
vi.mock("./toastStore", () => ({
  useToastStore: { getState: vi.fn(() => ({ show: vi.fn() })) },
}));
vi.mock("../permissions/rules", () => ({
  assessCommand: vi.fn(() => null),
}));

import { ipc } from "../ipc/tauri";
import { parseYesNo, speechFor, useVoiceStore } from "./voiceStore";

const mocked = vi.mocked(ipc);

function req(over: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "r1",
    ts: 1,
    sessionDir: "/home/u/.claude/projects/x",
    cwd: "/repo",
    tool: "Bash",
    input: {},
    ...over,
  };
}

describe("parseYesNo (a spoken word decides an approval — parse conservatively)", () => {
  it("understands yes in both languages, diacritics stripped", () => {
    for (const t of ["sí", "si", "Sí, dale", "yes", "approve", "APRUEBO", "ok"]) {
      expect(parseYesNo(t), t).toBe("yes");
    }
  });

  it("understands no", () => {
    for (const t of ["no", "No.", "cancela", "deny", "para"]) {
      expect(parseYesNo(t), t).toBe("no");
    }
  });

  it("a negative anywhere beats a positive — 'no, dale' must not approve", () => {
    expect(parseYesNo("no, dale")).toBe("no");
    expect(parseYesNo("dale... no")).toBe("no");
  });

  it("silence, ambiguity, or substrings stay unclear (modal stays in charge)", () => {
    expect(parseYesNo("")).toBe("unclear");
    expect(parseYesNo("quizás")).toBe("unclear");
    // "sinónimo" contains "si" but is not the word "si".
    expect(parseYesNo("sinónimo")).toBe("unclear");
  });
});

describe("speechFor (what gets read aloud)", () => {
  it("prefers the Bash description over the raw command", () => {
    const r = req({ input: { command: "rm -rf ./tmp", description: "Clean temp dir" } });
    expect(speechFor(r, false)).toBe("The agent wants to run: Clean temp dir");
  });

  it("truncates long commands so TTS does not read a wall of text", () => {
    const cmd = "x".repeat(300);
    const out = speechFor(req({ input: { command: cmd } }), true);
    expect(out).toContain("…");
    expect(out.length).toBeLessThan(160);
  });

  it("speaks file operations by basename, localized", () => {
    const r = req({ tool: "Write", input: { file_path: "/repo/src/app.ts" } });
    expect(speechFor(r, true)).toBe("El agente quiere escribir el archivo app.ts");
    expect(speechFor(r, false)).toBe("The agent wants to write the file app.ts");
  });

  it("falls back to the tool name for anything else", () => {
    const r = req({ tool: "WebFetch", input: {} });
    expect(speechFor(r, false)).toBe("The agent wants to use the WebFetch tool");
  });
});

describe("voiceStore state machine", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useVoiceStore.setState({ phase: "off", progress: null, approvalStage: null, error: null });
    vi.stubGlobal("window", { dispatchEvent: vi.fn() });
  });

  it("toggle from off enables voice and records the backend language", async () => {
    mocked.voiceEnable.mockResolvedValue({ language: "en" } as never);
    await useVoiceStore.getState().toggle();
    expect(useVoiceStore.getState().phase).toBe("ready");
    expect(useVoiceStore.getState().lang).toBe("en");
  });

  it("toggle failure lands back in off with the error kept", async () => {
    mocked.voiceEnable.mockRejectedValue(new Error("no mic"));
    await useVoiceStore.getState().toggle();
    const s = useVoiceStore.getState();
    expect(s.phase).toBe("off");
    expect(s.error).toContain("no mic");
  });

  it("toggle from ready disables even if the backend call fails", async () => {
    useVoiceStore.setState({ phase: "ready" });
    mocked.voiceDisable.mockRejectedValue(new Error("gone"));
    await useVoiceStore.getState().toggle();
    expect(useVoiceStore.getState().phase).toBe("off");
  });

  it("pttStart only arms from ready, and never during a spoken approval", async () => {
    await useVoiceStore.getState().pttStart();
    expect(mocked.voicePttStart).not.toHaveBeenCalled();

    useVoiceStore.setState({ phase: "ready", approvalStage: "speaking" });
    await useVoiceStore.getState().pttStart();
    expect(mocked.voicePttStart).not.toHaveBeenCalled();

    useVoiceStore.setState({ phase: "ready", approvalStage: null });
    mocked.voicePttStart.mockResolvedValue(undefined as never);
    await useVoiceStore.getState().pttStart();
    expect(useVoiceStore.getState().phase).toBe("listening");
  });

  it("pttStop types the transcript into the active session with a trailing space", async () => {
    useVoiceStore.setState({ phase: "listening" });
    mocked.voicePttStop.mockResolvedValue("  hola mundo  " as never);
    await useVoiceStore.getState().pttStop();

    expect(useVoiceStore.getState().phase).toBe("ready");
    const dispatched = vi.mocked(window.dispatchEvent).mock.calls[0][0] as CustomEvent;
    expect(dispatched.type).toBe("ac:term-input");
    expect(dispatched.detail).toEqual({ sessionId: "term-1", data: "hola mundo " });
  });

  it("pttStop with an empty transcript types nothing", async () => {
    useVoiceStore.setState({ phase: "listening" });
    mocked.voicePttStop.mockResolvedValue("   " as never);
    await useVoiceStore.getState().pttStop();
    expect(window.dispatchEvent).not.toHaveBeenCalled();
    expect(useVoiceStore.getState().phase).toBe("ready");
  });

  it("pttCancel drops the hold without typing", async () => {
    useVoiceStore.setState({ phase: "listening" });
    mocked.voicePttStop.mockResolvedValue("texto que no debe salir" as never);
    await useVoiceStore.getState().pttCancel();
    expect(window.dispatchEvent).not.toHaveBeenCalled();
    expect(useVoiceStore.getState().phase).toBe("ready");
  });
});
