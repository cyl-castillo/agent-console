import type { StorybookConfig } from "@storybook/react-vite";
import { fileURLToPath } from "node:url";
import { mergeConfig } from "vite";

const shim = (name: string) =>
  fileURLToPath(new URL(`../src/dev/tauri-shim/${name}.ts`, import.meta.url));

// Every Tauri package the frontend imports is swapped for a browser shim so
// the app renders in Chromium with fixture data (src/dev/fixtures). The
// production build never sees these aliases.
const tauriAliases = [
  { find: /^@tauri-apps\/api\/core$/, replacement: shim("core") },
  { find: /^@tauri-apps\/api\/event$/, replacement: shim("event") },
  { find: /^@tauri-apps\/api\/window$/, replacement: shim("window") },
  { find: /^@tauri-apps\/api\/webview$/, replacement: shim("webview") },
  { find: /^@tauri-apps\/api\/app$/, replacement: shim("app") },
  { find: /^@tauri-apps\/api\/path$/, replacement: shim("path") },
  { find: /^@tauri-apps\/plugin-opener$/, replacement: shim("plugin-opener") },
  { find: /^@tauri-apps\/plugin-dialog$/, replacement: shim("plugin-dialog") },
  { find: /^@tauri-apps\/plugin-process$/, replacement: shim("plugin-process") },
  { find: /^@tauri-apps\/plugin-updater$/, replacement: shim("plugin-updater") },
  {
    find: /^@tauri-apps\/plugin-clipboard-manager$/,
    replacement: shim("plugin-clipboard-manager"),
  },
  { find: /^@tauri-apps\/plugin-notification$/, replacement: shim("plugin-notification") },
];

const config: StorybookConfig = {
  framework: "@storybook/react-vite",
  stories: ["../src/stories/**/*.stories.@(ts|tsx)"],
  addons: ["@storybook/addon-a11y"],
  core: { disableTelemetry: true },
  viteFinal: (config) =>
    mergeConfig(config, {
      resolve: { alias: tauriAliases },
      // The app's Vite config pins port 1420 for Tauri; Storybook has its own.
      server: { port: undefined, strictPort: false },
    }),
};

export default config;
