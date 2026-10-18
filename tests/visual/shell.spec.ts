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

  // The drawer is closed but the PTY stays mounted.
  const drawer = page.getByTestId("terminal-drawer");
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
