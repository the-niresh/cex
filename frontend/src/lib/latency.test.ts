import { describe, expect, test } from "vitest";
import { LatencyWindow } from "./latency";

describe("LatencyWindow", () => {
  test("reports null until a sample carries a server number", () => {
    const w = new LatencyWindow(50);
    expect(w.stats().engineP50).toBeNull();
    expect(w.stats().networkP50).toBeNull();
  });

  test("a missing header is not a fast sample", () => {
    // An absent x-cex-server-us means the CORS exposure is wrong or the
    // response came from something that is not the API. Rendering that as 0ms
    // would advertise an exchange faster than physics.
    const w = new LatencyWindow(50);
    w.add(120, null);
    expect(w.stats().engineP50).toBeNull();
    expect(w.stats().count).toBe(0);
  });

  test("network is total minus server", () => {
    const w = new LatencyWindow(50);
    w.add(200, 40_000); // 200ms total, 40ms server
    const s = w.stats();
    expect(s.engineP50).toBe(40);
    expect(s.networkP50).toBe(160);
  });

  test("network never goes negative", () => {
    // Clock skew between performance.now() and the server's own measurement
    // can make the server number the larger of the two on a fast local call.
    const w = new LatencyWindow(50);
    w.add(5, 9_000);
    expect(w.stats().networkP50).toBe(0);
  });

  test("the window forgets samples past its capacity", () => {
    const w = new LatencyWindow(3);
    for (const ms of [100, 100, 100, 1, 1, 1]) w.add(ms, 1_000);
    expect(w.stats().count).toBe(3);
    expect(w.stats().networkP50).toBe(0);
  });

  test("p99 picks the worst of a hundred rather than the typical one", () => {
    const w = new LatencyWindow(100);
    for (let i = 0; i < 99; i++) w.add(10, 1_000);
    w.add(500, 1_000);
    const s = w.stats();
    expect(s.networkP50).toBe(9);
    expect(s.networkP99).toBe(499);
  });
});
