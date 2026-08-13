import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The API and WS crates listen on 8080/8081, only reachable as "localhost"
// from whatever machine is actually running the browser — which is not this
// one when the dev server is reached over SSH port forwarding. Proxying them
// through this same origin means only 5173 ever needs to be forwarded.
//
// The API's routes are top-level rather than under one `/api` prefix, so this
// has to name them. That is a list which can silently fall behind
// `crates/api/src/routes.rs` — so `src/lib/proxy.test.ts` reads both and fails
// when a route is added without being listed here. Keep it exported.
export const apiPaths = [
  "/health",
  "/register",
  "/login",
  "/deposit",
  "/balances",
  "/orders",
  "/markets",
  "/depth",
  "/trades",
  "/candles",
];

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      ...Object.fromEntries(apiPaths.map((path) => [path, "http://localhost:8080"])),
      "/ws": { target: "ws://localhost:8081", ws: true },
    },
  },
  test: {
    // `e2e/` belongs to Playwright. Without this vitest collects those specs
    // too, calls Playwright's `test()` outside a Playwright run, and `npm
    // test` fails on files it was never meant to execute.
    exclude: ["e2e/**", "node_modules/**", "dist/**"],
  },
});
