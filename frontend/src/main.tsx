import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
// Fonts are bundled, not fetched — a chart that reflows because a CDN was slow
// is a worse first impression than a plain one.
//
// Two families and two weights each. Inter carries the interface; Plex Mono
// carries the numbers, because digits have to line up down a ladder. The
// reference sets everything in Inter and gets away with it by showing far fewer
// numbers per screen than an order book does.
import "./theme.css";

const root = document.getElementById("root");
if (!root) throw new Error("no #root to mount into");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
