import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "@fontsource/martian-mono/500.css";
import "@fontsource/martian-mono/700.css";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("no #root to mount into");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
