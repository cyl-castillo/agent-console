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
