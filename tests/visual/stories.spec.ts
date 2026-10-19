// Screenshot + accessibility sweep over every Storybook story. Baselines for
// `toHaveScreenshot` live beside this file (git-ignored: Chromium and system
// fonts differ per machine); Argos does the cross-machine comparison in CI.
import { test, expect, type Page } from "@playwright/test";
import { argosScreenshot } from "@argos-ci/playwright";
import AxeBuilder from "@axe-core/playwright";
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

interface IndexEntry {
  id: string;
  title: string;
  name: string;
  type: "story" | "docs";
}

const SNAPSHOTS = process.env.VISUAL_SNAPSHOTS === "1";
const OUT = join(HERE, "out");
const A11Y_DIR = join(OUT, "a11y");

async function loadIndex(baseURL: string): Promise<IndexEntry[]> {
  const res = await fetch(`${baseURL}/index.json`);
  const json = (await res.json()) as { entries: Record<string, IndexEntry> };
  return Object.values(json.entries).filter((e) => e.type === "story");
}

/// Fixture timestamps are relative to this instant (src/dev/fixtures/defaults):
/// freezing the page clock keeps "4m ago" and countdowns identical run to run.
const FIXED_NOW = 1792245600000;

async function openStory(page: Page, id: string) {
  const errors: string[] = [];
  const warnings: string[] = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => {
    // Aborted external requests surface as resource-load errors; those are
    // listed in out/external-requests.txt instead of failing the story.
    if (m.type() === "error" && !m.text().startsWith("Failed to load resource"))
      errors.push(`console.error: ${m.text()}`);
    if (m.type() === "warning") warnings.push(m.text());
  });
  await page.clock.setFixedTime(new Date(FIXED_NOW));
  // Offline by construction: nothing but the static Storybook is reachable
  // (the updater's GitHub check would otherwise 403 under rate limiting).
  await page.route(/^(?!http:\/\/127\.0\.0\.1)/, (route) => {
    appendFileSync(join(OUT, "external-requests.txt"), `${id}: ${route.request().url()}\n`);
    void route.abort();
  });
  await page.goto(`/iframe.html?id=${id}&viewMode=story`);
  await page.locator("body[data-story-ready='1']").waitFor({ timeout: 15_000 });
  await page.evaluate(() => document.fonts.ready);
  return { errors, warnings };
}

test.describe("stories", () => {
  test("index is reachable", async ({ baseURL }) => {
    const live = await loadIndex(baseURL!);
    expect(live.map((e) => e.id).sort()).toEqual(entries.map((e) => e.id).sort());
  });

  // Enumerate at collection time from the static build on disk, so each
  // story is its own test (parallel, individually retryable, named in CI).
  const indexPath = join(HERE, "..", "..", "storybook-static", "index.json");
  const entries = Object.values(
    (JSON.parse(readFileSync(indexPath, "utf8")) as { entries: Record<string, IndexEntry> })
      .entries,
  ).filter((e) => e.type === "story");

  for (const e of entries) {
    test(e.id, async ({ page }) => {
      const { errors, warnings } = await openStory(page, e.id);
      expect(errors, `story ${e.id} logged errors`).toEqual([]);
      const missing = warnings
        .filter((w) => w.startsWith("[fixture] no fixture"))
        .map((w) => w.replace(/^\[fixture\] no fixture for "([^"]+)".*$/, "$1"));
      if (missing.length)
        appendFileSync(join(OUT, "missing-fixtures.txt"), `${e.id}: ${missing.join(", ")}\n`);
      await argosScreenshot(page, e.id, {
        fullPage: false,
        // A loading Button is legitimately aria-busy; the story is settled by data-story-ready.
        stabilize: { waitForAriaBusy: false },
      });
      if (SNAPSHOTS && process.env.VISUAL_BROWSER !== "webkit")
        await expect(page).toHaveScreenshot(`${e.id}.png`);
      if (process.env.VISUAL_BROWSER === "webkit")
        await page.screenshot({ path: join(OUT, "results-webkit", `${e.id}.png`) });
      // Layout gate (UI review 2026-10-15): the shell never scrolls as a
      // document, and nothing pokes out of the viewport unless it sits inside
      // a container that scrolls or clips on purpose. Overlays (modals,
      // popovers) are positioned by their own rules and are skipped.
      if (e.id.startsWith("app-v2--") || e.id.startsWith("app--")) {
        const layout = await page.evaluate(() => {
          const doc = document.documentElement;
          const vw = window.innerWidth;
          const vh = window.innerHeight;
          const clips = (el: Element): boolean => {
            for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
              const o = getComputedStyle(p);
              if (/(auto|scroll|hidden|clip)/.test(`${o.overflowX} ${o.overflowY}`)) return true;
            }
            return false;
          };
          const out: string[] = [];
          for (const el of Array.from(document.querySelectorAll("#root *"))) {
            const r = el.getBoundingClientRect();
            if (r.width === 0 || r.height === 0) continue;
            const cs = getComputedStyle(el);
            if (
              cs.position === "fixed" ||
              el.closest("[role='dialog'],[data-popover],.modal,[data-base-ui-popup]")
            )
              continue;
            if (
              (r.bottom > vh + 1 || r.right > vw + 1 || r.left < -1 || r.top < -1) &&
              !clips(el)
            ) {
              out.push(
                `${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ""}.${String(el.className).split(" ").slice(0, 3).join(".")} ` +
                  `[${Math.round(r.left)},${Math.round(r.top)}→${Math.round(r.right)},${Math.round(r.bottom)}]`,
              );
            }
            if (out.length >= 8) break;
          }
          return {
            docScroll: [doc.scrollHeight, doc.clientHeight, doc.scrollWidth, doc.clientWidth],
            out,
          };
        });
        expect(
          layout.docScroll[0] <= layout.docScroll[1] + 1 &&
            layout.docScroll[2] <= layout.docScroll[3] + 1,
          `story ${e.id}: the document scrolls (${layout.docScroll.join("/")})`,
        ).toBe(true);
        expect(layout.out, `story ${e.id}: elements outside the viewport`).toEqual([]);
      }
      // The Storybook a11y addon may still be mid-run inside the preview on
      // slow CI runners ("Axe is already running"); wait it out, don't fail.
      const axe = await (async () => {
        for (let attempt = 0; ; attempt++) {
          try {
            // The terminal's screen is the CLI's own output in the CLI's own
            // colours (the 256-colour cube is xterm's, not themeable); the
            // contract covers the console's chrome around it.
            return await new AxeBuilder({ page }).exclude(".xterm-screen").analyze();
          } catch (err) {
            if (attempt >= 4 || !String(err).includes("already running")) throw err;
            await page.waitForTimeout(400);
          }
        }
      })();
      const violations = axe.violations.map((v) => ({
        id: v.id,
        impact: v.impact,
        nodes: v.nodes.length,
        samples: v.nodes.slice(0, 3).map((n) => ({ target: n.target, summary: n.failureSummary })),
      }));
      writeFileSync(join(A11Y_DIR, `${e.id}.json`), JSON.stringify(violations, null, 2));
      // The primitives are the a11y contract: nothing serious or critical,
      // ever — and since F6 the Sessions stories too. Other app stories only
      // report (their debt is tracked by the audit).
      if (
        e.id.startsWith("primitives--") ||
        e.id.startsWith("app-v2--sessions") ||
        e.id === "app-v2--light-sessions"
      ) {
        const blocking = violations.filter(
          (v) => v.impact === "serious" || v.impact === "critical",
        );
        expect(blocking, `a11y violations in ${e.id}`).toEqual([]);
      }
    });
  }
});
