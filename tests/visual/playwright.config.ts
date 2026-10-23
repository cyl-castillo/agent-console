import { defineConfig, devices } from "@playwright/test";

const port = 6006;
// VISUAL_BROWSER=webkit runs the same sweep on Playwright's Linux WebKit —
// the closest engine to the app's WebKitGTK webview (UI review 2026-10-15).
// Pixel baselines stay Chromium-only; WebKit runs the behavioural specs and
// the layout gate, and writes its own screenshots under out/.
const webkit = process.env.VISUAL_BROWSER === "webkit";

export default defineConfig({
  testDir: ".",
  globalSetup: "./global-setup.ts",
  outputDir: webkit ? "out/results-webkit" : "out/results",
  fullyParallel: true,
  workers: process.env.CI ? 2 : 4,
  // One retry on CI only: the F3 cards spec flaked three times on unrelated
  // PRs (the card vanished mid-click under runner load) and passes locally
  // every time. The retry keeps a trace (see `use.trace`) so the next flake
  // leaves evidence in the uploaded results instead of a red check.
  retries: process.env.CI ? 1 : 0,
  reporter: [
    ["list"],
    // Argos uploads the screenshots for cross-run comparison when ARGOS_TOKEN
    // is set (CI); without it the reporter is a no-op.
    ["@argos-ci/playwright/reporter", { uploadToArgos: Boolean(process.env.ARGOS_TOKEN) }],
  ],
  use: {
    trace: "on-first-retry",
    baseURL: `http://127.0.0.1:${port}`,
    ...(webkit ? devices["Desktop Safari"] : devices["Desktop Chrome"]),
    // Argos' recommended flags: no subpixel AA or hinting differences between
    // machines, so screenshots compare across runners.
    launchOptions: webkit ? {} : { args: ["--disable-lcd-text", "--font-render-hinting=none"] },
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 1,
    colorScheme: "dark",
    locale: "en-US",
    timezoneId: "UTC",
  },
  expect: {
    toHaveScreenshot: { animations: "disabled", caret: "hide", maxDiffPixels: 0 },
  },
  webServer: {
    command: `node tests/visual/serve.mjs storybook-static ${port}`,
    url: `http://127.0.0.1:${port}/index.json`,
    reuseExistingServer: !process.env.CI,
    cwd: "../..",
  },
});
