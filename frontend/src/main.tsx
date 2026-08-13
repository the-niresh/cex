import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/martian-mono/500.css";
import "@fontsource/martian-mono/700.css";
// theme.css pulls in the stylesheet being replaced, as a cascade layer — see
// the note at the top of that file for why the order matters.
import "./theme.css";

const root = document.getElementById("root");
if (!root) throw new Error("no #root to mount into");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
