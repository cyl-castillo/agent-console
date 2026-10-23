import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The OpenCode witness plugin runs INSIDE OpenCode (Bun), not in the app;
// it is shipped as a resource and installed by hooks_service. This test
// drives it with the payloads captured from OpenCode 1.19.3 and checks that
// the event lines it appends are the ones hooks_service already understands.
// `node:fs` is mocked so the test sees exactly what the plugin would append.
import raw from "../../src-tauri/fixtures/hooks/opencode-1.19.3-plugin-raw.jsonl?raw";
import {
  AgentConsoleWitness,
  type WitnessHooks,
} from "../../src-tauri/resources/opencode-plugin.js";

const fs = vi.hoisted(() => ({ lines: [] as string[], exists: true }));
vi.mock("node:fs", () => ({
  appendFileSync: (_path: string, line: string) => {
    fs.lines.push(line);
  },
  existsSync: () => fs.exists,
}));

// The app's tsconfig has no Node types (it targets the webview); vitest runs
// this file under Node, where `process` exists.
declare const process: { env: Record<string, string | undefined> };

type Line = {
  kind?: string;
  input?: unknown;
  output?: unknown;
  parts?: unknown;
  event?: unknown;
};
type Ev = Record<string, unknown> & {
  type?: string;
  excerpt?: string;
  excerptTail?: string;
};

const RAW: Line[] = raw
  .trim()
  .split("\n")
  .map((l: string) => JSON.parse(l) as Line);

async function replay(hooks: WitnessHooks, lines = RAW) {
  for (const r of lines) {
    if (r.kind === "chat.message") await hooks["chat.message"]?.(r.input, { parts: r.parts });
    else if (r.kind === "tool.execute.after")
      await hooks["tool.execute.after"]?.(r.input, r.output);
    else if (r.kind === "event") await hooks.event?.({ event: r.event });
  }
}

const events = (): Ev[] => fs.lines.map((l) => JSON.parse(l.trim()) as Ev);

describe("OpenCode witness plugin", () => {
  const saved = {
    dir: process.env.AGENT_CONSOLE_SESSION_DIR,
    term: process.env.AGENT_CONSOLE_TERM_ID,
  };
  beforeEach(() => {
    fs.lines = [];
    fs.exists = true;
    process.env.AGENT_CONSOLE_SESSION_DIR = "/tmp/ac-session";
    process.env.AGENT_CONSOLE_TERM_ID = "term-9";
  });
  afterEach(() => {
    if (saved.dir === undefined) delete process.env.AGENT_CONSOLE_SESSION_DIR;
    else process.env.AGENT_CONSOLE_SESSION_DIR = saved.dir;
    if (saved.term === undefined) delete process.env.AGENT_CONSOLE_TERM_ID;
    else process.env.AGENT_CONSOLE_TERM_ID = saved.term;
  });

  it("is inert outside an Agent Console terminal", async () => {
    delete process.env.AGENT_CONSOLE_SESSION_DIR;
    expect(Object.keys(await AgentConsoleWitness({ directory: "/home/u/proj" }))).toEqual([]);
    process.env.AGENT_CONSOLE_SESSION_DIR = "/tmp/ac-session";
    fs.exists = false;
    expect(Object.keys(await AgentConsoleWitness({ directory: "/home/u/proj" }))).toEqual([]);
  });

  it("turns the real 1.19.3 payloads into prompt, tool results with exit codes, and a turn end", async () => {
    const hooks = await AgentConsoleWitness({ directory: "/home/u/proj" });
    await replay(hooks);
    const out = events();
    expect(out.map((e) => e.type)).toEqual([
      "user_prompt",
      "tool_result",
      "tool_result",
      "tool_result",
      "turn_end",
    ]);
    const prompt = out[0];
    expect(prompt.prompt).toContain("npm run test:fail");
    expect(prompt.sessionId).toBe("ses_e95b4caa1ffeDKYZHm9HVqo7e7");
    expect(prompt.termId).toBe("term-9");
    expect(prompt.cwd).toBe("/home/u/proj");
    expect(prompt.model).toBe("opencode/big-pickle");
    expect(prompt.observedVia).toBe("plugin");
    // The bash tool's exit code rides in metadata.exit: a failed `npm test`
    // is a failure here even though OpenCode has no failure event.
    const fail = out[1];
    expect(fail.tool).toBe("bash");
    expect(fail.command).toBe("npm test");
    expect(fail.exitCode).toBe(254);
    expect(fail.exitCodeVia).toBe("plugin");
    expect(fail.failed).toBe(true);
    expect(fail.toolUseId).toBe("call_00_1twaw84fsmv7p92g1g3ltp69");
    expect(fail.outputSha256).toMatch(/^[0-9a-f]{64}$/);
    expect(fail.excerpt).toContain("ENOENT");
    expect(out[3].command).toBe("npm run test:empty");
    expect(out[4].sessionId).toBe("ses_e95b4caa1ffeDKYZHm9HVqo7e7");
  });

  it("marks exit 0 as not failed, keeps a long output's tail, and closes the turn with the agent's words", async () => {
    const hooks = await AgentConsoleWitness({ directory: "/home/u/proj" });
    const sid = "ses_x";
    await hooks["chat.message"]?.(
      { sessionID: sid, model: { providerID: "opencode", modelID: "big-pickle" } },
      { parts: [{ type: "text", text: "run the tests" }] },
    );
    const body = "x".repeat(1200) + "\nTests:       0 total";
    await hooks["tool.execute.after"]?.(
      { tool: "bash", sessionID: sid, callID: "call_1", args: { command: "npx jest" } },
      { title: "npx jest", output: body, metadata: { exit: 0, output: body, truncated: false } },
    );
    await hooks.event?.({
      event: {
        type: "message.updated",
        properties: { sessionID: sid, info: { id: "msg_a", role: "assistant" } },
      },
    });
    await hooks.event?.({
      event: {
        type: "message.part.updated",
        properties: {
          sessionID: sid,
          part: { id: "prt_1", messageID: "msg_a", type: "text", text: "All tests pass." },
        },
      },
    });
    // A user message's text parts are not the agent's words.
    await hooks.event?.({
      event: {
        type: "message.part.updated",
        properties: {
          sessionID: sid,
          part: { id: "prt_u", messageID: "msg_user", type: "text", text: "nope" },
        },
      },
    });
    await hooks.event?.({ event: { type: "session.idle", properties: { sessionID: sid } } });
    // A second idle without a prompt in between is not another turn.
    await hooks.event?.({ event: { type: "session.idle", properties: { sessionID: sid } } });
    const out = events();
    expect(out.map((e) => e.type)).toEqual(["user_prompt", "tool_result", "turn_end"]);
    const tool = out[1];
    expect(tool.exitCode).toBe(0);
    expect(tool.failed).toBeUndefined();
    expect(tool.truncated).toBe(true);
    expect(tool.excerpt?.length).toBe(1000);
    expect(tool.excerptTail?.endsWith("Tests:       0 total")).toBe(true);
    expect(out[2].summary).toBe("All tests pass.");
    expect(out[2].summaryTruncated).toBe(false);
  });
});
