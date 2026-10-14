import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { installAgentBridge } from "./lib/agentBridge";
// Bundled fonts (fontsource, woff2 per unicode range): what the tokens name first.
import "@fontsource-variable/inter/opsz.css";
import "@fontsource-variable/jetbrains-mono";
import "./styles/index.css";

installAgentBridge();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
