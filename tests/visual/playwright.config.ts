import { defineConfig, devices } from "@playwright/test";

const port = 6006;

export default defineConfig({
  testDir: ".",
  globalSetup: "./global-setup.ts",
  outputDir: "out/results",
  fullyParallel: true,
  workers: process.env.CI ? 2 : 4,
  retries: 0,
  reporter: [
    ["list"],
    // Argos uploads the screenshots for cross-run comparison when ARGOS_TOKEN
    // is set (CI); without it the reporter is a no-op.
    ["@argos-ci/playwright/reporter", { uploadToArgos: Boolean(process.env.ARGOS_TOKEN) }],
  ],
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    ...devices["Desktop Chrome"],
    // Argos' recommended flags: no subpixel AA or hinting differences between
    // machines, so screenshots compare across runners.
    launchOptions: { args: ["--disable-lcd-text", "--font-render-hinting=none"] },
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
