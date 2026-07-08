import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, readTheme } from "./lib/theme";
import "./styles/global.css";

// Stamp the persisted theme before first paint so a saved theme doesn't flash
// the default Charcoal palette for a frame.
applyTheme(readTheme());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
