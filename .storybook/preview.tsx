import type { Preview } from "@storybook/react-vite";
import React from "react";
// The app's fonts plus the documented alternatives, for the Foundations specimens.
import "@fontsource-variable/inter/opsz.css";
import "@fontsource-variable/jetbrains-mono";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "../src/styles/index.css";

// The app sizes itself from #root (height: 100%); Storybook's root is not
// sized, so give every story the same box Tauri gives the window.
const frame: React.CSSProperties = { height: "100vh", width: "100vw", overflow: "hidden" };

const preview: Preview = {
  parameters: {
    layout: "fullscreen",
    backgrounds: { disable: true },
    // The panel runs axe on demand only: an automatic run collides with the
    // Playwright sweep's own axe pass ("Axe is already running").
    a11y: { test: "todo", manual: true },
  },
  decorators: [(Story) => <div style={frame}>{Story()}</div>],
};

export default preview;
