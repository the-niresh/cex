import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
// Fonts are bundled, not fetched — a chart that reflows because a CDN was slow
// is a worse first impression than a plain one. Two families, not three: the
// old stylesheet set labels in Martian Mono, and the type split moved them to
// Plex Mono's own sans sibling, so those two weights were being downloaded and
// never drawn.
import "./theme.css";

const root = document.getElementById("root");
if (!root) throw new Error("no #root to mount into");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
