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
