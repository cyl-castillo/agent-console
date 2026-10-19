// Behavioural check of the shell wiring the screenshots cannot see: the
// window events the palette and voice commands dispatch must still reach the
// (now store-backed) shell after the App.tsx extraction.
import { test, expect, type Page } from "@playwright/test";

const fire = (page: Page, name: string, detail?: unknown) =>
  page.evaluate(
    ([n, d]) => window.dispatchEvent(new CustomEvent(n as string, { detail: d })),
    [name, detail],
  );

test("palette window events drive the shell", async ({ page }) => {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto("/iframe.html?id=app-v2--sessions&viewMode=story");
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });

  await fire(page, "ac:open-tab", "changes");
  await expect(page.getByRole("tab", { name: /Changes/ })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("inspector")).toContainText("File");
  await fire(page, "ac:open-tab", "terminal");
  await expect(page.getByRole("tab", { name: "Conversation" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // Ctrl+E / "compose prompt" focuses the conversation's draft.
  await fire(page, "ac:toggle-composer");
  await expect(page.getByRole("textbox", { name: /prompt/i })).toBeFocused();

  await expect(page.getByTestId("sessions-list")).toBeVisible();
  await fire(page, "ac:toggle-sidebar");
  await expect(page.getByTestId("sessions-list")).toBeHidden();
  await page.getByRole("button", { name: "Getting started guide" }).click();
  await expect(page.getByText(/getting started/i).first()).toBeVisible();
});

const WORKBENCH_TABS = [
  "turns",
  "proof",
  "skills",
  "advisor",
  "learning",
  "context",
  "roundtable",
  "jobs",
  "schedule",
  "permissions",
  "vault",
  "plugins",
  "mcp",
  "jira",
  "teams",
  "agenda",
  "notes",
  "transfer",
  "feedback",
];

test("every workbench panel is reachable by deep link (UI-2026-10 F2 gate)", async ({ page }) => {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto("/iframe.html?id=app-v2--inbox&viewMode=story");
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
  const shell = page.getByTestId("shell-v2");
  for (const tab of WORKBENCH_TABS) {
    await fire(page, "ac:open-workbench-tab", tab);
    const dest = tab === "turns" || tab === "proof" ? "proof" : null;
    if (dest) await expect(shell).toHaveAttribute("data-destination", "proof");
    else {
      await expect(shell).not.toHaveAttribute("data-destination", "inbox");
      await expect(page.getByTestId("center-panel")).toBeVisible();
      await expect(page.getByTestId(`list-${tab}`)).toHaveAttribute("aria-current", "true");
    }
  }
});

test("v2 shell: rail, palette deep links and the decisions signal", async ({ page }) => {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto("/iframe.html?id=app-v2--sessions&viewMode=story");
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
  const shell = page.getByTestId("shell-v2");
  await expect(shell).toHaveAttribute("data-destination", "sessions");
  await expect(page.getByTestId("inspector")).toContainText("Turns");

  await page.getByTestId("rail-work").click();
  await expect(shell).toHaveAttribute("data-destination", "work");
  await expect(page.getByTestId("center-panel")).toContainText("Jobs");

  await page.getByTestId("list-notes").click();
  await expect(page.getByTestId("center-panel")).toContainText("Notes");

  // A palette deep link to a settings tab pulls the shell to Settings.
  await fire(page, "ac:open-workbench-tab", "vault");
  await expect(shell).toHaveAttribute("data-destination", "settings");
  await expect(page.getByTestId("center-panel")).toContainText("Vault");

  await page.getByTestId("sb-decisions").click();
  await expect(shell).toHaveAttribute("data-destination", "inbox");
  await expect(page.getByTestId("inbox")).toBeVisible();

  // Workspace button / palette toggle collapse the list pane in this shell.
  await page.getByTestId("rail-settings").click();
  await expect(page.getByTestId("list-pane")).toBeVisible();
  await fire(page, "ac:toggle-sidebar");
  await expect(page.getByTestId("list-pane")).toBeHidden();
});

test("v2 inbox: tiers gate the decision, deny removes the card", async ({ page }) => {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto("/iframe.html?id=app-v2--inbox-decisions&viewMode=story");
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
  const cards = page.locator("[data-testid^='decision-']");
  await expect(cards).toHaveCount(4);

  // D14: reaching outside the session perimeter is high tier, whatever the tool.
  await expect(page.locator("[data-testid='decision-apr-0045']")).toHaveAttribute(
    "data-tier",
    "high",
  );

  // High tier: Approve stays disabled until the word is typed.
  const high = page.locator("[data-testid='decision-apr-0043']");
  await expect(high).toHaveAttribute("data-tier", "high");
  const approve = high.getByRole("button", { name: "Approve once" });
  await expect(approve).toBeDisabled();
  await high.getByLabel(/Type approve/).fill("approve");
  await expect(approve).toBeEnabled();

  // Low tier: no typed word; Deny answers the hook and the card leaves.
  const low = page.locator("[data-testid='decision-apr-0042']");
  await expect(low).toHaveAttribute("data-tier", "low");
  await low.getByRole("button", { name: "Deny" }).click();
  await expect(cards).toHaveCount(3);
  await expect(page.getByTestId("budget-sess-auth")).toBeVisible();
});

test("v2 sessions: the conversation is the stage, the terminal a drawer", async ({ page }) => {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto("/iframe.html?id=app-v2--sessions&viewMode=story");
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
  await expect(page.getByTestId("conversation")).toBeVisible();
  await expect(page.getByTestId("turn")).toHaveCount(3);
  await expect(page.getByTestId("turn").last()).toHaveAttribute("data-live", "true");

  // The drawer is open by default (terminal first); Ctrl+` hides it and
  // the PTY stays mounted either way.
  const drawer = page.getByTestId("terminal-drawer");
  await expect(drawer).toHaveAttribute("data-open", "true");
  await expect(drawer.locator(".terminals-stack > *")).toHaveCount(1);
  await page.keyboard.press("Control+`");
  await expect(drawer).not.toHaveAttribute("data-open", "true");
  await expect(drawer.locator(".terminals-stack > *")).toHaveCount(1);
  await page.keyboard.press("Control+`");
  await expect(drawer).toHaveAttribute("data-open", "true");

  // Sending from the composer types into the active session's PTY: the text,
  // then Enter as its own keystroke. Plain Enter sends (Shift+Enter adds a
  // line), and the prompt is echoed at once so the click never looks dead.
  await page.evaluate(() => {
    const w = window as unknown as { __termInputs: { sessionId: string; data: string }[] };
    w.__termInputs = [];
    window.addEventListener("ac:term-input", (e) =>
      w.__termInputs.push((e as CustomEvent<{ sessionId: string; data: string }>).detail),
    );
  });
  const prompt = page.getByLabel("Prompt for the agent");
  await prompt.fill("Add a changelog");
  await page.keyboard.press("Shift+Enter");
  await prompt.type(" entry");
  await expect(prompt).toHaveValue("Add a changelog\n entry");
  await prompt.fill("Add a changelog entry");
  await page.keyboard.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __termInputs: { data: string }[] }).__termInputs.map((d) => d.data),
      ),
    )
    .toEqual(["Add a changelog entry", "\r"]);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __termInputs: { sessionId: string }[] }).__termInputs[0].sessionId,
    ),
  ).toBe("sess-auth");
  await expect(page.getByTestId("pending-prompt")).toContainText("Add a changelog entry");
  await expect(prompt).toHaveValue("");
  await expect(prompt).toBeFocused();

  // A file chip opens Changes at that file.
  await page.getByTestId("diff-block").first().getByRole("button").first().click();
  await expect(page.getByRole("tab", { name: /Changes/ })).toHaveAttribute("aria-selected", "true");
});

test("the conversation follows the ledger live: an appended line shows without a new prompt", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  // The live turn is still "working" in the story's ledger.
  await expect(page.getByTestId("conversation")).toContainText("Working…");
  // The Rust side appends the turn's close and announces it AFTER the write.
  await page.evaluate(async () => {
    const w = window as unknown as {
      __fixtures: {
        setFixture: (c: string, v: unknown) => void;
        emit: (n: string, p: unknown) => Promise<void>;
        getFixture: (c: string) => unknown;
      };
    };
    const base = w.__fixtures.getFixture("testigo_list") as {
      seq: number;
      turnId?: string;
      termId?: string;
      kind: string;
    }[];
    const live = [...base].reverse().find((e) => e.kind === "prompt" && e.termId === "sess-auth");
    const close = {
      seq: 900,
      ts: Date.now(),
      caseId: "case-2026-10-13",
      turnId: live?.turnId,
      kind: "turn_end",
      termId: "sess-auth",
      actor: "agent",
      payload: { summary: "Backoff is now configurable through RetryOptions.", filesChanged: [] },
      prevHash: "",
      hash: "",
    };
    w.__fixtures.setFixture("testigo_list", [...base, close]);
    await w.__fixtures.emit("ledger://appended", {
      projectRoot: "/home/demo/code/demo-playground",
      seq: 900,
      kind: "turn_end",
      termId: "sess-auth",
    });
  });
  await expect(page.getByTestId("conversation")).toContainText(
    "Backoff is now configurable through RetryOptions.",
    { timeout: 3000 },
  );
  await expect(page.getByTestId("conversation")).not.toContainText("Working…");
});

test("Ctrl+Enter queues for later; the prompt goes out on its own when the turn ends", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  await page.evaluate(() => {
    const w = window as unknown as { __termInputs: { data: string }[] };
    w.__termInputs = [];
    window.addEventListener("ac:term-input", (e) =>
      w.__termInputs.push((e as CustomEvent<{ data: string }>).detail),
    );
  });
  const prompt = page.getByLabel("Prompt for the agent");
  await expect(page.getByTestId("conversation")).toContainText("Working…");
  await prompt.fill("Then add the changelog entry");
  await page.keyboard.press("Control+Enter");
  // Queued, visible, not sent: the turn is still live.
  await expect(page.getByTestId("prompt-queue")).toContainText("Then add the changelog entry");
  await expect(page.getByTestId("prompt-queue")).toContainText("next, when this turn ends");
  await expect(prompt).toHaveValue("");
  expect(
    await page.evaluate(
      () => (window as unknown as { __termInputs: unknown[] }).__termInputs.length,
    ),
  ).toBe(0);

  // The turn ends (the ledger appends its close and announces it).
  await page.evaluate(async () => {
    const w = window as unknown as {
      __fixtures: {
        setFixture: (c: string, v: unknown) => void;
        emit: (n: string, p: unknown) => Promise<void>;
        getFixture: (c: string) => unknown;
      };
    };
    const base = w.__fixtures.getFixture("testigo_list") as {
      turnId?: string;
      termId?: string;
      kind: string;
    }[];
    const live = [...base].reverse().find((e) => e.kind === "prompt" && e.termId === "sess-auth");
    w.__fixtures.setFixture("testigo_list", [
      ...base,
      {
        seq: 901,
        ts: Date.now(),
        caseId: "case-2026-10-13",
        turnId: live?.turnId,
        kind: "turn_end",
        termId: "sess-auth",
        actor: "agent",
        payload: { summary: "Done.", filesChanged: [] },
        prevHash: "",
        hash: "",
      },
    ]);
    await w.__fixtures.emit("ledger://appended", {
      projectRoot: "/home/demo/code/demo-playground",
      seq: 901,
      kind: "turn_end",
      termId: "sess-auth",
    });
  });
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __termInputs: { data: string }[] }).__termInputs.map((d) => d.data),
      ),
    )
    .toEqual(["Then add the changelog entry", "\r"]);
  await expect(page.getByTestId("prompt-queue")).toHaveCount(0);
  await expect(page.getByTestId("pending-prompt")).toContainText("Then add the changelog entry");
});

test("v2 proof: the ledger map selects a turn into the inspector", async ({ page }) => {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto("/iframe.html?id=app-v2--proof-ledger&viewMode=story");
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
  await expect(page.getByTestId("ledger-minimap")).toBeVisible();
  await expect(page.getByTestId("inspector")).toBeHidden();
  await page.getByTestId("minimap-turn-turn-002").click();
  await expect(page.getByTestId("inspector")).toContainText("Turn");
  await expect(page.getByTestId("turn-inspector")).toContainText("turn-002");
  await expect(page.getByTestId("turn-inspector")).toContainText("commit");
});

test("v2 sessions list: grouped by attention, WIP visible, files underneath", async ({ page }) => {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto("/iframe.html?id=app-v2--sessions&viewMode=story");
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
  const list = page.getByTestId("sessions-list");
  await expect(list).toContainText("Stopped");
  await expect(page.getByTestId("session-sess-retry")).toContainText("ac/retry-path");
  await page.getByRole("button", { name: "Files" }).click();
  await expect(list).toContainText("package.json");
  await page.getByTestId("sessions-new").click();
  await expect(page.getByRole("listbox", { name: "Choose an agent and model" })).toBeVisible();
});

async function openV2(page: Page, story: string) {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto(`/iframe.html?id=${story}&viewMode=story`);
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
}

test("a queued prompt waits for a stopped session instead of looping", async ({ page }) => {
  await openV2(page, "app-v2--sessions");
  await page.evaluate(() => {
    const w = window as unknown as { __termInputs: { data: string }[] };
    w.__termInputs = [];
    window.addEventListener("ac:term-input", (e) =>
      w.__termInputs.push((e as CustomEvent<{ data: string }>).detail),
    );
  });
  const prompt = page.getByLabel("Prompt for the agent");
  await expect(page.getByTestId("conversation")).toContainText("Working…");
  await prompt.fill("Then add the changelog entry");
  await page.keyboard.press("Control+Enter");
  await expect(page.getByTestId("prompt-queue")).toContainText("next, when this turn ends");

  // The PTY exits (the session is "stopped"), then the turn closes. Before
  // the fix, a failed delivery re-queued the prompt and re-ran the sender
  // forever: the webview froze.
  await page.evaluate(async () => {
    const w = window as unknown as {
      __fixtures: {
        invokeLog: { cmd: string; args: Record<string, unknown> }[];
        setFixture: (c: string, v: unknown) => void;
        emit: (n: string, p: unknown) => Promise<void>;
        getFixture: (c: string) => unknown;
      };
    };
    for (const s of w.__fixtures.invokeLog.filter((i) => i.cmd === "term_spawn")) {
      await w.__fixtures.emit("term://exit", {
        id: `term-${String(s.args.termKey ?? "x")}`,
        code: 0,
      });
    }
    const base = w.__fixtures.getFixture("testigo_list") as {
      turnId?: string;
      termId?: string;
      kind: string;
    }[];
    const live = [...base].reverse().find((e) => e.kind === "prompt" && e.termId === "sess-auth");
    w.__fixtures.setFixture("testigo_list", [
      ...base,
      {
        seq: 901,
        ts: Date.now(),
        caseId: "case-2026-10-13",
        turnId: live?.turnId,
        kind: "turn_end",
        termId: "sess-auth",
        actor: "agent",
        payload: { summary: "Done.", filesChanged: [] },
        prevHash: "",
        hash: "",
      },
    ]);
    await w.__fixtures.emit("ledger://appended", {
      projectRoot: "/home/demo/code/demo-playground",
      seq: 901,
      kind: "turn_end",
      termId: "sess-auth",
    });
  });
  await expect(page.getByTestId("conversation")).toContainText("Done.");
  // Still queued, labelled, nothing typed, and the page answers.
  await expect(page.getByTestId("prompt-queue")).toContainText("waiting for the session");
  await page.waitForTimeout(500);
  expect(
    await page.evaluate(
      () => (window as unknown as { __termInputs: unknown[] }).__termInputs.length,
    ),
  ).toBe(0);
  await expect(page.getByTestId("prompt-queue").locator("li")).toHaveCount(1);
});

test("terminal first: open by default at 60 %, foldable chat, resizable and remembered", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  const centre = page.getByTestId("sessions-center");
  const drawer = page.getByTestId("terminal-drawer");
  await expect(drawer).toHaveAttribute("data-open", "true");
  const ratio = async () => {
    const c = (await centre.boundingBox())!;
    const d = (await drawer.boundingBox())!;
    return d.height / c.height;
  };
  expect(await ratio()).toBeGreaterThan(0.55);
  expect(await ratio()).toBeLessThan(0.65);

  // Hide the chat: the terminal takes the whole centre; show it back.
  await page.getByTestId("toggle-chat").click();
  await expect(drawer).toHaveAttribute("data-full", "true");
  await expect(page.getByTestId("conversation")).toBeHidden();
  expect(await ratio()).toBeGreaterThan(0.97);
  await page.getByTestId("toggle-chat").click();
  await expect(page.getByTestId("conversation")).toBeVisible();

  // Drag the strip up by a quarter of the centre: the drawer grows and the
  // new size is remembered; double-click resets.
  const strip = page.getByTestId("drawer-strip");
  const s = (await strip.boundingBox())!;
  const c = (await centre.boundingBox())!;
  await page.mouse.move(s.x + 40, s.y + s.height / 2);
  await page.mouse.down();
  await page.mouse.move(s.x + 40, s.y + s.height / 2 - c.height * 0.2, { steps: 6 });
  await page.mouse.up();
  expect(await ratio()).toBeGreaterThan(0.75);
  const saved = await page.evaluate(
    () => JSON.parse(localStorage.getItem("agent-console:shell-v2") ?? "{}").drawerPct,
  );
  expect(saved).toBeGreaterThan(75);
  await strip.dblclick();
  expect(await ratio()).toBeGreaterThan(0.55);
  expect(await ratio()).toBeLessThan(0.65);
});

const termOut = (page: Page, id: string, data: string) =>
  page.evaluate(
    ([i, d]) =>
      (
        window as unknown as { __fixtures: { emit: (n: string, p: unknown) => Promise<void> } }
      ).__fixtures.emit("term://output", { id: i, data: d }),
    [id, data],
  );

test("F3: the trust question, a login ask and a missing CLI surface as cards with a way in", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  // The story's drawer is open by default; hide it so the cards' CTA is tested.
  await page.keyboard.press("Control+`");
  await expect(page.getByTestId("terminal-drawer")).not.toHaveAttribute("data-open", "true");

  await termOut(
    page,
    "term-sess-auth",
    "\x1b[1mQuick safety check:\x1b[0m Is this a project you created or one you trust?\r\n",
  );
  const trust = page.getByTestId("card-trust");
  await expect(trust).toContainText("trust this folder");
  await trust.getByRole("button", { name: "Answer in the terminal" }).click();
  await expect(page.getByTestId("terminal-drawer")).toHaveAttribute("data-open", "true");
  // The CLI comes up: the question is behind us.
  await termOut(page, "term-sess-auth", "✻ Welcome to Claude Code\r\n❯ ");
  await expect(trust).toHaveCount(0);

  await termOut(page, "term-sess-auth", "Please run /login to continue\r\n");
  await expect(page.getByTestId("card-login")).toContainText("log in");
  await expect(
    page.getByTestId("card-login").getByRole("button", { name: "Log in" }),
  ).toBeVisible();

  await termOut(page, "term-sess-auth", "bash: claude: command not found\r\n");
  await expect(page.getByTestId("card-notfound")).toContainText("not installed");
  await page.getByTestId("card-notfound").getByRole("button", { name: "Open setup" }).click();
  await expect(page.getByText("Welcome to Agent Console")).toBeVisible();
});

test("F5: the agent's words stream into the live turn before it closes, as real markdown", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  await expect(page.getByTestId("live-status")).toContainText("Working…");
  // The conversation asked the Rust side to follow this session's transcript.
  const watched = await page.evaluate(() => {
    const log = (
      window as unknown as {
        __fixtures: { invokeLog: { cmd: string; args: Record<string, unknown> }[] };
      }
    ).__fixtures.invokeLog;
    return log.find((e) => e.cmd === "transcript_watch")?.args;
  });
  expect(watched).toEqual({ sessionId: "0f2c1e4a-9b7d-4c1e-8a2f-5d6e7f8a9b0c", engine: "claude" });
  const emit = (items: unknown[]) =>
    page.evaluate(
      (payload) =>
        (
          window as unknown as { __fixtures: { emit: (n: string, p: unknown) => Promise<void> } }
        ).__fixtures.emit("transcript://delta", payload),
      {
        sessionId: "0f2c1e4a-9b7d-4c1e-8a2f-5d6e7f8a9b0c",
        engine: "claude",
        backfill: false,
        items,
      },
    );
  // A tool call with no result yet: the turn says what is running.
  await emit([
    { kind: "tool_use", id: "t1", ts: Date.now(), name: "Bash", input: "npm test -- --run" },
  ]);
  await expect(page.getByTestId("live-status")).toContainText("Running Bash");
  await expect(page.getByTestId("live-status")).toContainText("npm test -- --run");
  // The agent writes: its paragraph appears while the turn is still open.
  await emit([
    { kind: "tool_result", id: "t1", ts: Date.now(), ok: true },
    {
      kind: "text",
      id: "a1",
      ts: Date.now(),
      text: "The **auth** test passes now. See [src/auth.ts](src/auth.ts).\n\n| file | status |\n|---|---|\n| auth.ts | ok |\n\n```ts\nconst ok = true;\n```\n\n<img src=x onerror=alert(1)>",
    },
  ]);
  const live = page.getByTestId("turn").last();
  const agent = live.getByTestId("agent-block");
  await expect(agent).toContainText("Agent");
  await expect(agent.locator("strong")).toHaveText("auth");
  await expect(agent.getByTestId("md-table")).toBeVisible();
  await expect(agent.getByTestId("code-fence")).toHaveAttribute("data-lang", "ts");
  await expect(agent.getByTestId("md-path")).toHaveText("src/auth.ts");
  // 0 HTML from the model: the tag is text, not an element.
  await expect(agent.locator("img")).toHaveCount(0);
  await expect(agent).toContainText("<img src=x onerror=alert(1)>");
  // Still live: no turn_end, no proof line, labels on both sides.
  await expect(live).toHaveAttribute("data-live", "true");
  await expect(live.getByTestId("proof-block")).toHaveCount(0);
  await expect(live.getByTestId("you-block")).toContainText("You");
  await expect(page.getByTestId("live-status")).toContainText("Working…");
  // The path opens Changes.
  await agent.getByTestId("md-path").click();
  await expect(page.getByRole("tab", { name: /Changes/ })).toHaveAttribute("aria-selected", "true");
});

test("F3: an approval of this session sits inline in its conversation and the turn says so", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  await expect(page.getByTestId("live-status")).toContainText("Working…");
  await page.evaluate(() =>
    (
      window as unknown as { __fixtures: { emit: (n: string, p: unknown) => Promise<void> } }
    ).__fixtures.emit("approval://request", {
      id: "apr-7001",
      ts: Date.now(),
      sessionDir: "/home/demo/.claude",
      cwd: "/home/demo/code/demo-playground",
      tool: "Bash",
      input: { command: "npm test -- --run", description: "Run the tests" },
      termId: "sess-auth",
      source: "permission_request",
    }),
  );
  // Inline, in the thread; the live turn is waiting, not working; no modal.
  const card = page.getByTestId("inline-approvals").locator("[data-testid='decision-apr-7001']");
  await expect(card).toBeVisible();
  await expect(page.getByTestId("live-status")).toContainText("Waiting for your approval");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await card.getByRole("button", { name: "Approve once" }).click();
  await expect(card).toHaveCount(0);
  await expect(page.getByTestId("live-status")).toContainText("Working…");
  const decided = await page.evaluate(() => {
    const log = (
      window as unknown as {
        __fixtures: { invokeLog: { cmd: string; args: Record<string, unknown> }[] };
      }
    ).__fixtures.invokeLog;
    return log.find((l) => l.cmd === "approval_respond" && l.args.id === "apr-7001")?.args;
  });
  expect(decided).toMatchObject({ decision: "allow" });
});

test("F3: while the CLI boots, the thread says so and the composer queues instead of typing", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions-booting");
  await expect(page.getByTestId("card-boot")).toContainText("Launching");
  await page.evaluate(() => {
    const w = window as unknown as { __termInputs: { data: string }[] };
    w.__termInputs = [];
    window.addEventListener("ac:term-input", (e) =>
      w.__termInputs.push((e as CustomEvent<{ data: string }>).detail),
    );
  });
  const prompt = page.getByLabel("Prompt for the agent");
  await prompt.fill("Start by running the tests");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("prompt-queue")).toContainText("Start by running the tests");
  expect(
    await page.evaluate(
      () => (window as unknown as { __termInputs: unknown[] }).__termInputs.length,
    ),
  ).toBe(0);
  // The prompt comes up: the card goes and the queued prompt goes out.
  await termOut(page, "term-sess-auth", "\x1b[38;5;215m✻ Welcome to Claude Code\x1b[0m\r\n❯ ");
  await expect(page.getByTestId("card-boot")).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __termInputs: { data: string }[] }).__termInputs.map((d) => d.data),
      ),
    )
    .toEqual(["Start by running the tests", "\r"]);
});

test("F4: coming back — the last session is selected, a click resumes it with its history", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions-all-stopped");
  // Every session stopped; nothing remembered yet → nothing selected.
  await expect(page.getByTestId("resume-banner")).toHaveCount(0);
  const stopped = page.getByTestId("sessions-list").getByText("fix auth test");
  await stopped.click();
  // Resume: the PTY spawns and the history is there without any prompt.
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as { __fixtures: { invokeLog: { cmd: string }[] } }
          ).__fixtures.invokeLog.filter((l) => l.cmd === "term_spawn").length,
      ),
    )
    .toBeGreaterThan(0);
  await expect(page.getByTestId("turn")).toHaveCount(3);
  await expect(page.getByTestId("resume-banner")).toHaveCount(0);
  // The choice is remembered for the next launch.
  const remembered = await page.evaluate(() =>
    localStorage.getItem("agent-console:active-session:/home/demo/code/demo-playground"),
  );
  expect(remembered).toBe("sess-auth");
});

test("F4: the composer on a stopped session resumes it and the prompt goes out when the CLI is up", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions-all-stopped");
  await page.evaluate(() => {
    (window as unknown as { __activate: () => void }).__activate = () => undefined;
    const w = window as unknown as { __termInputs: { data: string }[] };
    w.__termInputs = [];
    window.addEventListener("ac:term-input", (e) =>
      w.__termInputs.push((e as CustomEvent<{ data: string }>).detail),
    );
  });
  // Select without resuming: the Inbox's "Open session" of a stopped one resumes,
  // so pick it through the bridge state instead — the row click would resume.
  await page.getByTestId("sessions-list").getByText("retry path regression").click();
  await expect(page.getByTestId("resume-banner")).toHaveCount(0);
  await page.evaluate(() => {
    // Stop it again to stand in the "stopped, selected" state the user comes back to.
    const w = window as unknown as {
      __fixtures: { emit: (n: string, p: unknown) => Promise<void> };
    };
    return w.__fixtures.emit("term://exit", { id: "term-sess-retry", code: 0 });
  });
  await expect(page.getByTestId("resume-banner")).toBeVisible();
  const prompt = page.getByLabel("Prompt for the agent");
  await expect(prompt).toHaveAttribute("placeholder", /resumes the session/);
  await prompt.fill("Pick up where we left off");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("prompt-queue")).toContainText("Pick up where we left off");
  // The CLI comes back (the fixture's banner) and the prompt goes out on its own.
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __termInputs: { data: string }[] }).__termInputs.map((d) => d.data),
      ),
    )
    .toEqual(["Pick up where we left off", "\r"]);
  await expect(page.getByTestId("resume-banner")).toHaveCount(0);
});

test("F4: first launch with nothing to decide lands in Sessions", async ({ page }) => {
  await openV2(page, "app-v2--landing-fresh");
  await expect(page.getByTestId("shell-v2")).toHaveAttribute("data-destination", "sessions");
});

test("F4: the + and Ctrl+T open the same chooser; Enter starts with the last agent", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  await page.getByTestId("sessions-new").click();
  await expect(page.getByTestId("session-chooser")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("session-chooser")).toHaveCount(0);
  // Ctrl+T from anywhere outside a field or the terminal.
  await page.getByTestId("rail-inbox").click();
  await page.locator("body").click({ position: { x: 5, y: 400 } });
  await page.keyboard.press("Control+t");
  await expect(page.getByTestId("shell-v2")).toHaveAttribute("data-destination", "sessions");
  await expect(page.getByTestId("session-chooser")).toBeVisible();
  const before = await page.evaluate(() => window.__agentConsole.state().sessions.length);
  await page.keyboard.press("Enter");
  await expect
    .poll(async () => page.evaluate(() => window.__agentConsole.state().sessions.length))
    .toBe(before + 1);
  await expect(page.getByTestId("session-chooser")).toHaveCount(0);
});

test("focus mode: Ctrl+Shift+F leaves the terminal alone; the pill, the strip and the palette leave it", async ({
  page,
}) => {
  await openV2(page, "app-v2--inbox");
  await page.locator("body").click({ position: { x: 5, y: 400 } });
  await page.keyboard.press("Control+Shift+f");
  const shell = page.getByTestId("shell-v2");
  await expect(shell).toHaveAttribute("data-focus", "true");
  await expect(shell).toHaveAttribute("data-destination", "sessions");
  await expect(page.getByTestId("rail")).toBeHidden();
  await expect(page.getByTestId("sessions-list")).toBeHidden();
  await expect(page.getByTestId("inspector")).toBeHidden();
  await expect(page.getByTestId("conversation")).toBeHidden();
  await expect(page.getByTestId("terminal-drawer")).toHaveAttribute("data-full", "true");
  await expect(page.getByTestId("terminal-drawer")).toBeVisible();
  // Nothing but the status bar around the terminal: the drawer spans the shell.
  const shellBox = (await shell.boundingBox())!;
  const drawerBox = (await page.getByTestId("terminal-drawer").boundingBox())!;
  expect(drawerBox.height).toBeGreaterThan(shellBox.height * 0.9);
  expect(drawerBox.width).toBeGreaterThan(shellBox.width * 0.98);

  await page.getByTestId("exit-focus").click();
  await expect(shell).not.toHaveAttribute("data-focus", "true");
  await expect(page.getByTestId("rail")).toBeVisible();
  await expect(page.getByTestId("conversation")).toBeVisible();

  // The strip button and the action contract are the other ways in and out.
  await page.getByTestId("toggle-focus").click();
  await expect(shell).toHaveAttribute("data-focus", "true");
  expect(await page.evaluate(() => window.__agentConsole.invoke("ui.focus_mode"))).toBe(true);
  await expect(shell).not.toHaveAttribute("data-focus", "true");
});

test("v2 parity: the terminal survives a trip to the Inbox", async ({ page }) => {
  await openV2(page, "app-v2--sessions-drawer-open");
  const drawer = page.getByTestId("terminal-drawer");
  await expect(drawer).toBeVisible();
  const xtermBefore = await page.locator(".xterm").count();
  expect(xtermBefore).toBeGreaterThan(0);

  await page.getByTestId("rail-inbox").click();
  await expect(page.getByTestId("inbox")).toBeVisible();
  // Hidden, not unmounted: the same xterm instances are still in the DOM.
  await expect(page.getByTestId("zone-sessions")).toBeHidden();
  await expect(drawer).toBeAttached();
  expect(await page.locator(".xterm").count()).toBe(xtermBefore);

  await page.getByTestId("rail-sessions").click();
  await expect(drawer).toBeVisible();
});

test("v2 parity: below 900 px the side columns collapse, the list can be summoned", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  await expect(page.getByTestId("inspector")).toBeVisible();
  await page.setViewportSize({ width: 860, height: 720 });
  const shell = page.getByTestId("shell-v2");
  await expect(shell).toHaveAttribute("data-narrow", "true");
  await expect(page.getByTestId("inspector")).toBeHidden();
  await expect(page.getByTestId("sessions-list")).toBeHidden();
  await fire(page, "ac:toggle-sidebar");
  await expect(page.getByTestId("sessions-list")).toBeVisible();
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(shell).not.toHaveAttribute("data-narrow", "true");
  await expect(page.getByTestId("inspector")).toBeVisible();
});

test("v2 parity: saved rooms live under Work and open the Room panel", async ({ page }) => {
  await openV2(page, "app-v2--work-jobs");
  const rooms = page.getByTestId("saved-rooms");
  await expect(rooms).toBeVisible();
  await rooms.locator(".session-row").first().click();
  await expect(page.getByTestId("center-panel")).toContainText("Room");
});

test("v2 parity: the inspector is drag-resizable and the width persists", async ({ page }) => {
  await openV2(page, "app-v2--sessions");
  const handle = page.getByTestId("resize-inspector");
  await expect(handle).toBeVisible();
  const box = (await handle.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x - 100, y, { steps: 5 });
  await page.mouse.up();
  const width = await page.evaluate(
    () => JSON.parse(localStorage.getItem("agent-console:shell-v2") ?? "{}").inspectorWidth,
  );
  expect(width).toBeGreaterThanOrEqual(450);
  const cols = await page
    .getByTestId("shell-v2")
    .evaluate((el) => getComputedStyle(el).gridTemplateColumns);
  expect(cols).toContain(`${width}px`);
  // Double-click resets to the default.
  await handle.dblclick();
  const reset = await page.evaluate(
    () => JSON.parse(localStorage.getItem("agent-console:shell-v2") ?? "{}").inspectorWidth,
  );
  expect(reset).toBe(360);
});

test("v2 parity: onboarding is folded into the Inbox", async ({ page }) => {
  await openV2(page, "app-v2--inbox-onboarding-hint");
  const hint = page.getByTestId("onboarding-hint");
  await expect(hint).toContainText(/getting-started step/);
  await hint.getByRole("button", { name: "Open guide" }).click();
  await expect(page.getByText(/getting started/i).first()).toBeVisible();
  await page.keyboard.press("Escape");
  await hint.getByRole("button", { name: "Don't show again" }).click();
  await expect(hint).toBeHidden();
});

test("D12 harness lab: declared metric, lineage, and a run the human merges", async ({ page }) => {
  await openV2(page, "app-v2--panel-learning");
  const lab = page.getByTestId("harness-lab");
  await expect(lab).toContainText("heldout_pass_rate");
  await expect(page.getByTestId("lab-stats")).toContainText(
    "2 experiments · 1 kept · 2 without touching the evaluator",
  );
  // Lineage and the PR link of the kept experiment are visible; nothing merges here.
  await expect(page.getByTestId("lab-experiments")).toContainText("↳ hx-1792000000-001");
  await expect(page.getByTestId("lab-experiments").getByRole("button", { name: "PR" })).toHaveCount(
    1,
  );
  await expect(lab.getByRole("button", { name: /merge/i })).toHaveCount(0);
  // Hidden tasks are edited only here (outside the repo).
  await lab.getByRole("button", { name: /edit hidden tasks/ }).click();
  await expect(page.getByTestId("lab-heldout")).toContainText("the agent never reads them");
  // Run now goes through the backend command.
  await lab.getByRole("button", { name: "Run experiment" }).click();
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const log = (
          window as unknown as {
            __fixtures: { invokeLog: { cmd: string; args: Record<string, unknown> }[] };
          }
        ).__fixtures.invokeLog;
        return log.some((l) => l.cmd === "harness_lab_run");
      }),
    )
    .toBe(true);
});

/// CHAT-2026-10 F1: the sidebar reads at thirty sessions. The name keeps the
/// row's width (actions live out of flow), rows carry no button chrome, each
/// group is newest-activity first, "working" belongs to the terminal that
/// earned it, and the rail badge counts what needs the human, not what is live.
test("F1 sidebar: names keep their width, rows are flat, recency orders, ✻ stays put", async ({
  page,
}) => {
  await page.clock.setFixedTime(new Date(1792245600000));
  await openV2(page, "app-v2--sessions-many");
  const list = page.getByTestId("sessions-list");
  const names = list.getByTestId("session-name");
  await expect(names).toHaveCount(30);
  for (const box of await names.evaluateAll((els) =>
    els.map((el) => el.getBoundingClientRect().width),
  )) {
    expect(box).toBeGreaterThanOrEqual(150);
  }
  const row = page.locator("[data-testid='session-sess-m09'] > button");
  await expect(row).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(row).toHaveCSS("border-top-width", "0px");

  // Groups: 1–3 working (open turn), 4–8 idle, the rest stopped — and each
  // group newest first, by (k · 7) mod 30 slots ago.
  const ids = (state: string) =>
    page
      .locator(`[data-testid='sessions-${state}'] [data-testid^='session-sess-']`)
      .evaluateAll((els) => els.map((el) => el.getAttribute("data-testid")!.slice(8)));
  const order = (ks: number[]) =>
    [...ks]
      .sort((a, b) => ((a * 7) % 30) - ((b * 7) % 30))
      .map((k) => `sess-m${String(k).padStart(2, "0")}`);
  expect(await ids("working")).toEqual(order([1, 2, 3]));
  expect(await ids("stopped")).toEqual(order(Array.from({ length: 22 }, (_, i) => i + 9)));
  await expect(
    page.locator("[data-testid='sessions-idle'] [data-testid^='session-sess-']"),
  ).toHaveCount(5);

  // The rail counts "needs you", not "live": eight live sessions, no badge.
  await expect(page.getByTestId("rail-sessions")).toHaveAttribute("aria-label", "Sessions");
  await expect(list).toContainText("Sessions8");

  // A hook event names its terminal: ✻ lands on that row and stays there
  // when another row is clicked.
  await page.evaluate(() =>
    (
      window as unknown as { __fixtures: { emit: (n: string, p: unknown) => Promise<void> } }
    ).__fixtures.emit("hook://user_prompt", {
      type: "user_prompt",
      ts: Date.now(),
      prompt: "Carry on",
      termId: "sess-m05",
    }),
  );
  await expect(page.getByTestId("session-sess-m05")).toHaveAttribute("data-state", "working");
  await page.getByTestId("session-sess-m06").getByRole("button").first().click();
  await expect(page.locator("[data-testid='session-sess-m06'] > button").first()).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(page.getByTestId("session-sess-m05")).toHaveAttribute("data-state", "working");
  await expect(page.getByTestId("session-sess-m06")).toHaveAttribute("data-state", "idle");
});

// ---- CHAT-2026-10 F2: the chat follows and answers -------------------------

type Fx = {
  __fixtures: {
    setFixture: (c: string, v: unknown) => void;
    emit: (n: string, p: unknown) => Promise<void>;
    getFixture: (c: string) => unknown;
  };
};
/// Append one ledger line for sess-auth and announce it, as the Rust side does.
const appendLedger = (page: Page, line: Record<string, unknown>) =>
  page.evaluate(async (line) => {
    const w = window as unknown as Fx;
    const base = w.__fixtures.getFixture("testigo_list") as { kind: string; termId?: string }[];
    w.__fixtures.setFixture("testigo_list", [...base, line]);
    await w.__fixtures.emit("ledger://appended", {
      projectRoot: "/home/demo/code/demo-playground",
      seq: line.seq,
      kind: line.kind,
      termId: "sess-auth",
    });
  }, line);
const liveTurnId = (page: Page) =>
  page.evaluate(() => {
    const base = (window as unknown as Fx).__fixtures.getFixture("testigo_list") as {
      kind: string;
      termId?: string;
      turnId?: string;
    }[];
    return [...base].reverse().find((e) => e.kind === "prompt" && e.termId === "sess-auth")?.turnId;
  });
const closeLine = (turnId: string | undefined, summary: string, seq = 900) => ({
  seq,
  ts: Date.now(),
  caseId: "case-2026-10-13",
  turnId,
  kind: "turn_end",
  termId: "sess-auth",
  actor: "agent",
  payload: { summary, summaryTruncated: summary.length >= 1000, filesChanged: [] },
  prevHash: "",
  hash: "",
});
const distanceFromEnd = (page: Page) =>
  page.evaluate(() => {
    const el = document.querySelector("[data-testid='conversation-log']") as HTMLElement;
    return el.scrollHeight - el.scrollTop - el.clientHeight;
  });
const LONG = "The agent wrote a long reply here. ".repeat(40);

test("F2 the conversation follows: a long reply and a send land at the end", async ({ page }) => {
  await openV2(page, "app-v2--sessions");
  expect(await distanceFromEnd(page)).toBeLessThanOrEqual(1);
  await appendLedger(page, closeLine(await liveTurnId(page), LONG));
  await expect(page.getByTestId("conversation")).toContainText("The agent wrote a long reply");
  await expect.poll(() => distanceFromEnd(page)).toBeLessThanOrEqual(1);
  await expect(page.getByTestId("new-reply-pill")).toHaveCount(0);
  // The cut summary says where the rest is, and takes you there.
  await page.getByTestId("summary-more").click();
  await expect(page.getByTestId("terminal-drawer")).toHaveAttribute("data-open", "true");
  await page.keyboard.press("Control+`");

  const prompt = page.getByLabel("Prompt for the agent");
  await prompt.fill("And now the changelog");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("pending-prompt")).toContainText("And now the changelog");
  await expect.poll(() => distanceFromEnd(page)).toBeLessThanOrEqual(1);
});

test("F2 away from the end, a reply raises a pill and the view stays put", async ({ page }) => {
  await openV2(page, "app-v2--sessions");
  await page.evaluate(() => {
    const el = document.querySelector("[data-testid='conversation-log']") as HTMLElement;
    el.scrollTop = 0;
  });
  await expect
    .poll(() =>
      page.evaluate(
        () => (document.querySelector("[data-testid='conversation-log']") as HTMLElement).scrollTop,
      ),
    )
    .toBe(0);
  await appendLedger(page, closeLine(await liveTurnId(page), LONG));
  await expect(page.getByTestId("new-reply-pill")).toBeVisible();
  expect(
    await page.evaluate(
      () => (document.querySelector("[data-testid='conversation-log']") as HTMLElement).scrollTop,
    ),
  ).toBe(0);
  await page.getByTestId("new-reply-pill").click();
  await expect.poll(() => distanceFromEnd(page)).toBeLessThanOrEqual(1);
  await expect(page.getByTestId("new-reply-pill")).toHaveCount(0);
});

test("F2 a prompt on an open turn ends it as interrupted; the new one is live", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  await expect(page.getByTestId("turn")).toHaveCount(3);
  await appendLedger(page, {
    seq: 902,
    ts: Date.now(),
    caseId: "case-2026-10-13",
    turnId: "turn-004",
    kind: "prompt",
    termId: "sess-auth",
    actor: "human",
    payload: { prompt: "Next thing, please" },
    prevHash: "",
    hash: "",
  });
  await expect(page.getByTestId("turn")).toHaveCount(4);
  await expect(page.getByTestId("turn").nth(2)).toContainText("interrupted");
  await expect(page.getByTestId("turn").nth(2)).not.toHaveAttribute("data-live", "true");
  await expect(page.getByTestId("turn").nth(3)).toHaveAttribute("data-live", "true");
  await expect(page.getByTestId("turn").nth(3)).toContainText("Next thing, please");
});

test("F2 Interrupt is taken at its word: the turn reads cut short and the queue goes out", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  await page.evaluate(() => {
    const w = window as unknown as { __termInputs: { data: string }[] };
    w.__termInputs = [];
    window.addEventListener("ac:term-input", (e) =>
      w.__termInputs.push((e as CustomEvent<{ data: string }>).detail),
    );
  });
  const prompt = page.getByLabel("Prompt for the agent");
  await prompt.fill("After that, the docs");
  await page.keyboard.press("Control+Enter");
  await expect(page.getByTestId("prompt-queue")).toContainText("next, when this turn ends");
  await page.getByTestId("interrupt").click();
  await expect(page.getByTestId("interrupt")).toContainText("Stopping…");
  await expect(page.getByTestId("interrupt")).toBeDisabled();
  // Claude sends no close: after the grace the turn reads as cut short, the
  // Interrupt button is gone and the queued prompt goes out on its own.
  await expect(page.getByTestId("turn").last()).toContainText("interrupted", { timeout: 8000 });
  await expect(page.getByTestId("interrupt")).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __termInputs: { data: string }[] }).__termInputs.map((d) => d.data),
      ),
    )
    .toEqual(["\x1b", "After that, the docs", "\r"]);
  await expect(page.getByTestId("prompt-queue")).toHaveCount(0);
});
