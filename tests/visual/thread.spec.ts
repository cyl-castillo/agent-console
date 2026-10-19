// CHAT-2026-10 F6 gates: one thread (the Inspector is the turn's detail),
// rename and undo in the sessions list, ↑/↓ between rows, and scale — two
// hundred turns render a screenful, idle without long tasks, with a bounded
// number of tab stops.
import { test, expect, type Page } from "@playwright/test";

const openV2 = async (page: Page, story: string) => {
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => void route.abort());
  await page.goto(`/iframe.html?id=${story}&viewMode=story`);
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
};

test("F6: 200 turns — a screenful in the DOM, no long tasks at rest, ≤ 40 tab stops", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions-long-thread");
  const log = page.getByTestId("conversation-log");
  await expect(log).toBeVisible();
  // End-anchored: the last turn is on screen, the first is not rendered.
  await expect(page.getByTestId("turn").last()).toContainText("(#200)");
  const rendered = await page.getByTestId("turn").count();
  expect(rendered).toBeLessThan(40);
  expect(rendered).toBeGreaterThan(0);
  await expect(page.getByTestId("turn-list")).toHaveCount(1);
  // Idle: two seconds without a task over 50 ms.
  const longTasks = await page.evaluate(
    () =>
      new Promise<number[]>((resolve) => {
        const seen: number[] = [];
        const obs = new PerformanceObserver((list) => {
          for (const e of list.getEntries()) seen.push(Math.round(e.duration));
        });
        obs.observe({ type: "longtask", buffered: false });
        setTimeout(() => {
          obs.disconnect();
          resolve(seen);
        }, 2000);
      }),
  );
  expect(
    longTasks.filter((d) => d > 50),
    "long tasks at rest",
  ).toEqual([]);
  // Tab stops visible in the centre: the thread's secondary actions are
  // behind "…", so the count stays bounded whatever the thread's length.
  const stops = await page.evaluate(() => {
    const root = document.querySelector('[data-testid="conversation"]');
    if (!root) return -1;
    const els = root.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select, [tabindex]:not([tabindex="-1"])',
    );
    return Array.from(els).filter(
      (el) => el.tabIndex >= 0 && el.offsetParent !== null && el.getClientRects().length > 0,
    ).length;
  });
  expect(stops).toBeLessThanOrEqual(40);
  // Scrolling to the top brings the first turn in and lets the last go.
  await log.evaluate((el) => (el.scrollTop = 0));
  await expect(page.getByTestId("turn").first()).toContainText("(#1)");
});

test("F6: the Inspector is the selected turn's detail, not the thread again", async ({ page }) => {
  await openV2(page, "app-v2--sessions");
  const inspector = page.getByTestId("inspector");
  await expect(inspector).toContainText("Turn");
  // By default: the last turn (the live one).
  await expect(inspector.getByTestId("detail-prompt")).toContainText(
    "Refactor retry.ts so backoff is configurable",
  );
  await expect(inspector.getByTestId("detail-tools")).toBeVisible();
  // "…" → Details on an earlier turn selects it.
  const first = page.getByTestId("turn").first();
  await first.getByTestId("turn-menu").click();
  await page.getByRole("menuitem", { name: "Details" }).click();
  await expect(first).toHaveAttribute("data-selected", "true");
  await expect(inspector.getByTestId("detail-prompt")).toContainText(
    "Fix the failing user-auth test",
  );
  await expect(inspector.getByTestId("detail-actions")).toContainText("Re-run");
  // The thread's secondary icons are out of the tab order; the menu is in.
  await expect(first.getByRole("button", { name: "Copy summary" })).toHaveAttribute(
    "tabindex",
    "-1",
  );
  await expect(first.getByTestId("turn-menu")).not.toHaveAttribute("tabindex", "-1");
});

test("F6: rename in place (double-click, F2), a menu per session, Remove with Undo", async ({
  page,
}) => {
  await openV2(page, "app-v2--sessions");
  // Rename the live, active session (a click on a stopped one resumes it).
  const row = page.getByTestId("session-sess-auth");
  await row.getByTestId("session-name").dblclick();
  const input = row.getByTestId("session-rename");
  await expect(input).toBeFocused();
  await input.fill("auth — fixed");
  await input.press("Enter");
  await expect(row.getByTestId("session-name")).toHaveText("auth — fixed");
  // F2 on the focused row, Escape keeps the name.
  await row.locator("[data-session-row]").focus();
  await page.keyboard.press("F2");
  await expect(row.getByTestId("session-rename")).toBeFocused();
  await page.keyboard.type("nope");
  await page.keyboard.press("Escape");
  await expect(row.getByTestId("session-name")).toHaveText("auth — fixed");
  // ↑/↓ move between rows.
  const retry = page.getByTestId("session-sess-retry");
  await row.locator("[data-session-row]").focus();
  await page.keyboard.press("ArrowDown");
  await expect(retry.locator("[data-session-row]")).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(row.locator("[data-session-row]")).toBeFocused();
  // Right-click: the same menu.
  await retry.click({ button: "right" });
  await expect(page.getByRole("menuitem", { name: "Rename" })).toBeVisible();
  await page.keyboard.press("Escape");
  // Remove hides the row at once; Undo brings it back.
  await retry.hover();
  await retry.getByTestId("session-menu").click();
  await page.getByRole("menuitem", { name: "Remove" }).click();
  await expect(retry).toHaveCount(0);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByTestId("session-sess-retry")).toBeVisible();
  const closed = await page.evaluate(() =>
    (
      window as unknown as { __fixtures: { invokeLog: { cmd: string; args: unknown }[] } }
    ).__fixtures.invokeLog
      .filter((e) => /^term_kill|forget|close/i.test(e.cmd))
      .map((e) => `${e.cmd} ${JSON.stringify(e.args)}`),
  );
  expect(closed).toEqual([]);
});
