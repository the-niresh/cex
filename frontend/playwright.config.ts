import { defineConfig } from "@playwright/test";

/**
 * Drives the real stack — engine, api, persist, ws — exactly as the Rust suites
 * do. There is no mock backend anywhere in here: a test that passes against a
 * fake is a test that proves nothing about whether the exchange works.
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  // A real engine, a real database and a real socket. Each of these does a
  // full register → deposit → place → cancel round trip against them, which
  // legitimately takes tens of seconds — the budget is generous on purpose so
  // a slow round trip is not reported as a broken screen.
  timeout: 150_000,
  expect: { timeout: 20_000 },
  reporter: [["list"]],
  use: {
    baseURL: "http://localhost:5173",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npx vite --port 5173 --strictPort",
    url: "http://localhost:5173",
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
