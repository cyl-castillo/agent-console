// Behavioural check of the shell wiring the screenshots cannot see: the
// window events the palette and voice commands dispatch must still reach the
// (now store-backed) shell after the App.tsx extraction.
import { test, expect, type Page } from "@playwright/test";

async function openTerminalStory(page: Page) {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto("/iframe.html?id=app--terminal&viewMode=story");
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
}

const fire = (page: Page, name: string, detail?: unknown) =>
  page.evaluate(
    ([n, d]) => window.dispatchEvent(new CustomEvent(n as string, { detail: d })),
    [name, detail],
  );

test("palette window events drive the shell", async ({ page }) => {
  await openTerminalStory(page);

  await fire(page, "ac:open-tab", "changes");
  await expect(page.locator(".tabs .tab.active")).toHaveText(/Changes/);
  await expect(page.locator(".panel.right .file-inspector, .panel.right")).toBeVisible();

  await fire(page, "ac:open-tab", "terminal");
  await expect(page.locator(".tabs .tab.active")).toHaveText(/Terminal/);

  await expect(page.locator(".sidebar-toggle.open")).toHaveCount(0);
  await fire(page, "ac:toggle-sidebar");
  await expect(page.locator(".sidebar-toggle.open")).toHaveCount(1);
  await expect(page.locator(".resize-handle.left")).toBeVisible();

  await fire(page, "ac:toggle-right-panel");
  await expect(page.locator(".app.right-collapsed")).toHaveCount(1);
  await fire(page, "ac:toggle-right-panel");
  await expect(page.locator(".app.right-collapsed")).toHaveCount(0);

  await fire(page, "ac:open-workbench-tab", "schedule");
  await expect(page.locator(".panel.right")).toContainText("Nightly held-out tests");

  await fire(page, "ac:open-shortcuts");
  await expect(page.getByText("Keyboard shortcuts", { exact: false }).first()).toBeVisible();
  await page.keyboard.press("Escape");

  await fire(page, "ac:toggle-composer");
  await expect(page.locator(".tabs .tab.active")).toHaveText(/Terminal/);

  const before = await page
    .locator(".terminals-stack .terminal-host, .terminals-stack > *")
    .count();
  await fire(page, "ac:new-session");
  await expect(page.locator(".terminals-stack > *")).toHaveCount(before + 1);
});

test("the top bar buttons open the overlays", async ({ page }) => {
  await openTerminalStory(page);
  await page.getByRole("button", { name: "About Agent Console" }).click();
  await expect(page.getByText("early preview")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Getting started guide" }).click();
  await expect(page.getByText(/getting started/i).first()).toBeVisible();
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
  await expect(cards).toHaveCount(3);

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
  await expect(cards).toHaveCount(2);
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

  // Sending from the composer types into the active session's PTY.
  const received = page.evaluate(
    () =>
      new Promise<string>((resolve) =>
        window.addEventListener("ac:term-input", (e) =>
          resolve((e as CustomEvent<{ sessionId: string; data: string }>).detail.sessionId),
        ),
      ),
  );
  await page.getByLabel("Prompt for the agent").fill("Add a changelog entry");
  await page.keyboard.press("Control+Enter");
  expect(await received).toBe("sess-auth");

  // A file chip opens Changes at that file.
  await page.getByTestId("diff-block").first().getByRole("button").first().click();
  await expect(page.getByRole("tab", { name: /Changes/ })).toHaveAttribute("aria-selected", "true");
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
