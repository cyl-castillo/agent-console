import type { Preview } from "@storybook/react-vite";
import React from "react";
import "../src/styles/global.css";
import "../src/styles/tailwind.css";

// The app sizes itself from #root (height: 100%); Storybook's root is not
// sized, so give every story the same box Tauri gives the window.
const frame: React.CSSProperties = { height: "100vh", width: "100vw", overflow: "hidden" };

const preview: Preview = {
  parameters: {
    layout: "fullscreen",
    backgrounds: { disable: true },
    a11y: { test: "todo" },
  },
  decorators: [(Story) => <div style={frame}>{Story()}</div>],
};

export default preview;
