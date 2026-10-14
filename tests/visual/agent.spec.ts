// The agent-operability gate (F6): an agent that only has the accessibility
// tree and the bridge must complete "start a session → decide → export a
// proof packet" in the v2 shell. No test ids, no CSS selectors: roles, names
// and labels only — what a computer-use agent or a screen reader gets.
import { test, expect, type Page } from "@playwright/test";

interface Bridge {
  listActions: () => { id: string; available: boolean }[];
  invoke: (id: string) => Promise<boolean>;
  state: () => {
    sessions: unknown[];
    pendingDecisions: number;
    destination: string;
    uiV2: boolean;
  };
}
declare global {
  interface Window {
    __agentConsole: Bridge;
  }
}

async function open(page: Page, story: string) {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto(`/iframe.html?id=${story}&viewMode=story`);
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
}
const state = (page: Page) => page.evaluate(() => window.__agentConsole.state());

test("an agent drives the console by roles, names and the bridge", async ({ page }) => {
  await open(page, "app-v2--inbox-decisions");

  // The bridge publishes the action contract and the console's state.
  const actions = await page.evaluate(() => window.__agentConsole.listActions().map((a) => a.id));
  expect(actions).toContain("nav.proof");
  expect(actions).toContain("session.new");
  expect((await state(page)).pendingDecisions).toBe(3); // the approval queue

  // Decide: the read-only request, by its card's words and the button's name.
  const readOnly = page.getByRole("article").filter({ hasText: "Read-only" }).first();
  await readOnly.getByRole("button", { name: "Approve once" }).click();
  await expect(page.getByRole("article").filter({ hasText: "Read-only" })).toHaveCount(0);
  expect((await state(page)).pendingDecisions).toBe(2);

  // A high-tier request refuses a bare click: the word must be typed first.
  const high = page.getByRole("article").filter({ hasText: "Irreversible" }).first();
  await expect(high.getByRole("button", { name: "Approve once" })).toBeDisabled();
  await high.getByLabel(/Type approve/).fill("approve");
  await expect(high.getByRole("button", { name: "Approve once" })).toBeEnabled();
  await high.getByRole("button", { name: "Approve once" }).click();
  // The edit: deny it. Leaving the Inbox with decisions pending would pop
  // the modal elsewhere — an agent clears its queue before moving on.
  await page
    .getByRole("article")
    .filter({ hasText: "Edits a file" })
    .first()
    .getByRole("button", { name: "Deny" })
    .click();
  await expect.poll(async () => (await state(page)).pendingDecisions).toBe(0);

  // Start a session from the Inbox header.
  const before = (await state(page)).sessions.length;
  await page.getByRole("button", { name: "New session" }).click();
  await expect.poll(async () => (await state(page)).sessions.length).toBe(before + 1);

  // Go to Proof through the contract, not the mouse; export the case's packet.
  expect(await page.evaluate(() => window.__agentConsole.invoke("nav.proof"))).toBe(true);
  await expect.poll(async () => (await state(page)).destination).toBe("proof");
  await page.getByRole("button", { name: "export", exact: true }).first().click();
  await expect(page.getByText(/review before signing/i)).toBeVisible();
  await page.getByTitle("Sign the packet with the chosen redactions").click();
  await expect(page.getByText(/review before signing/i)).toBeHidden();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("the console answers an agent's MCP call and names it as the actor", async ({ page }) => {
  await open(page, "app-v2--settings-mcp");
  // The panel tells the user how to point an agent at this console.
  await expect(page.getByTestId("console-mcp-section")).toContainText(
    "claude mcp add agent-console",
  );
  await page.getByLabel("Agent name").fill("claude-desktop");
  await expect(page.getByTestId("console-mcp-section")).toContainText("--caller claude-desktop");

  // The Rust side emits console://call; the webview answers via console_reply.
  const before = (await state(page)).sessions.length;
  await page.evaluate(() =>
    (
      window as unknown as { __fixtures: { emit: (n: string, p: unknown) => Promise<void> } }
    ).__fixtures.emit("console://call", {
      id: 42,
      caller: "claude-desktop",
      tool: "new_session",
      arguments: { name: "docs", agent: "claude" },
    }),
  );
  await expect.poll(async () => (await state(page)).sessions.length).toBe(before + 1);
  const reply = await page.evaluate(() => {
    const log = (
      window as unknown as {
        __fixtures: { invokeLog: { cmd: string; args: Record<string, unknown> }[] };
      }
    ).__fixtures.invokeLog;
    return log.find((l) => l.cmd === "console_reply")?.args;
  });
  expect(reply).toMatchObject({
    id: 42,
    result: { ok: true, result: { name: "docs", agent: "claude" } },
  });
  expect((await state(page)).sessions.some((s) => (s as { name: string }).name === "docs")).toBe(
    true,
  );
});
