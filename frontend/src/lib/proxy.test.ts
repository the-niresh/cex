import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { apiPaths } from "../../vite.config";

/**
 * The dev proxy names the API's routes one by one.
 *
 * It has to: `crates/api/src/routes.rs` mounts everything at the top level
 * (`/register`, `/depth/{symbol}`) rather than under one `/api` prefix, so
 * there is no single pattern to forward. A hand-written list of that shape
 * rots the day someone adds a route — and it rots *quietly*, because the new
 * route still works when hit directly and only 404s through the dev server.
 *
 * This is the alarm on that drift. It reads the real router and fails if a
 * route is not covered. The alternative — giving every API route an `/api`
 * prefix — changes the wire contract for every existing caller, which is a
 * much bigger change than a dev-only proxy justifies.
 */
const ROUTES_RS = fileURLToPath(new URL("../../../crates/api/src/routes.rs", import.meta.url));

/** Every path passed to `.route(...)` in the router, in source order. */
function routedPaths(source: string): string[] {
  return [...source.matchAll(/\.route\(\s*"([^"]+)"/g)].map((m) => m[1]!);
}

/** Vite matches a proxy key as a prefix of the request path. */
function isProxied(path: string): boolean {
  return apiPaths.some((prefix) => path === prefix || path.startsWith(`${prefix}/`));
}

describe("the dev proxy covers the API", () => {
  const paths = routedPaths(readFileSync(ROUTES_RS, "utf8"));

  it("finds the routes it is meant to be checking", () => {
    // Guards against the regex silently matching nothing after a refactor,
    // which would turn every assertion below into a vacuous pass.
    expect(paths).toContain("/register");
    expect(paths.length).toBeGreaterThan(8);
  });

  it.each(paths)("forwards %s", (path) => {
    expect(
      isProxied(path),
      `${path} is routed by crates/api/src/routes.rs but not proxied by vite.config.ts — ` +
        "add it to apiPaths or it will 404 through the dev server",
    ).toBe(true);
  });

  it("has no proxy entry that no longer matches a route", () => {
    // The other direction: a path left behind after a route was renamed.
    const stale = apiPaths.filter((prefix) => !paths.some((path) => path.startsWith(prefix)));
    expect(stale).toEqual([]);
  });
});
