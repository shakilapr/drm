// Renderer entry point (React 19 + Vite). REQ: REQ-STUDIO-011.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./index.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("missing #root container");
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
