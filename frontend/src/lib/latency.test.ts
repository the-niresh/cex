import { describe, expect, test } from "vitest";
import { LatencyWindow } from "./latency";

/** A request that reached the matching engine. */
const engineCall = (totalMs: number, serverUs: number, engineUs: number) =>
  [totalMs, serverUs, engineUs] as const;

describe("LatencyWindow", () => {
  test("reports null until a sample carries a server number", () => {
    const w = new LatencyWindow(50);
    expect(w.stats().engine.p50).toBeNull();
    expect(w.stats().network.p50).toBeNull();
    expect(w.stats().history.p50).toBeNull();
  });

  test("a missing header is not a fast sample", () => {
    // An absent x-cex-server-us means the CORS exposure is wrong or the
    // response came from something that is not the API. Rendering that as 0ms
    // would advertise an exchange faster than physics.
    const w = new LatencyWindow(50);
    w.add(120, null, null);
    expect(w.stats().engine.p50).toBeNull();
    expect(w.stats().network.count).toBe(0);
  });

  test("a server number without an engine number cannot be classified", () => {
    // Both headers ship together. One without the other means we cannot tell
    // an engine call from a history query, and guessing would put database
    // time under a label that says "engine".
    const w = new LatencyWindow(50);
    w.add(120, 40_000, null);
    expect(w.stats().network.count).toBe(0);
  });

  test("counts responses that arrived without usable timings", () => {
    // The case this exists for: a deployed API built before the timing
    // middleware answers every request perfectly well and reports nothing. All
    // three gauges then sit at "—", which is indistinguishable from a screen
    // nobody has used yet — so the count is what lets the strip say which.
    const w = new LatencyWindow(50);
    expect(w.stats().unreported).toBe(0);

    w.add(120, null, null);
    w.add(95, 40_000, null);
    expect(w.stats().unreported).toBe(2);
    expect(w.stats().engine.count).toBe(0);
  });

  test("stops reporting unusable responses once anything measures", () => {
    // One good sample proves the server reports, so the earlier misses are
    // noise rather than a diagnosis.
    const w = new LatencyWindow(50);
    w.add(120, null, null);
    w.add(...engineCall(200, 40_000, 30_000));
    expect(w.stats().unreported).toBe(0);
  });

  test("network is total minus the server's own time", () => {
    const w = new LatencyWindow(50);
    w.add(...engineCall(200, 40_000, 30_000));
    expect(w.stats().network.p50).toBe(160);
  });

  test("network never goes negative", () => {
    // performance.now() and the server's clock are not the same clock, so a
    // fast local call can invert them.
    const w = new LatencyWindow(50);
    w.add(...engineCall(5, 9_000, 8_000));
    expect(w.stats().network.p50).toBe(0);
  });

  describe("splitting engine from history", () => {
    // The whole point of the split. `/trades` and `/candles` are served from
    // Postgres and report x-cex-engine-us: 0 — they never reach the engine.
    // Averaging them together with real engine calls buries a 1ms matching
    // engine under a cloud database's round trip, and puts the blame for it
    // on the wrong subsystem.
    test("an engine call reports its engine time, not the whole request", () => {
      const w = new LatencyWindow(50);
      w.add(...engineCall(50, 3_000, 2_000));

      expect(w.stats().engine.p50).toBe(2);
      expect(w.stats().engine.count).toBe(1);
      expect(w.stats().history.count).toBe(0);
    });

    test("a request that never reached the engine counts as history", () => {
      const w = new LatencyWindow(50);
      w.add(900, 762_000, 0);

      expect(w.stats().history.p50).toBe(762);
      expect(w.stats().history.count).toBe(1);
      expect(w.stats().engine.count).toBe(0);
    });

    test("a slow history query cannot drag the engine reading up", () => {
      const w = new LatencyWindow(50);
      for (let i = 0; i < 9; i++) w.add(...engineCall(10, 2_000, 1_000));
      w.add(900, 762_000, 0);

      expect(w.stats().engine.p50).toBe(1);
      expect(w.stats().history.p50).toBe(762);
    });

    test("network counts both, because the wire does not care who served it", () => {
      const w = new LatencyWindow(50);
      w.add(...engineCall(100, 2_000, 1_000));
      w.add(900, 762_000, 0);

      expect(w.stats().network.count).toBe(2);
    });
  });

  test("the window forgets samples past its capacity", () => {
    const w = new LatencyWindow(3);
    for (const ms of [100, 100, 100, 1, 1, 1]) w.add(ms, 1_000, 500);
    expect(w.stats().network.count).toBe(3);
    expect(w.stats().network.p50).toBe(0);
  });

  test("p99 picks the worst of a hundred rather than the typical one", () => {
    const w = new LatencyWindow(100);
    for (let i = 0; i < 99; i++) w.add(...engineCall(10, 1_000, 500));
    w.add(...engineCall(500, 1_000, 500));
    const s = w.stats();
    expect(s.network.p50).toBe(9);
    expect(s.network.p99).toBe(499);
  });
});

describe("the sample series", () => {
  test("comes back oldest first", () => {
    const w = new LatencyWindow(50);
    w.add(...engineCall(100, 1_000, 1_000));
    w.add(...engineCall(200, 2_000, 2_000));

    expect(w.series().map((s) => s.engineMs)).toEqual([1, 2]);
    expect(w.series().map((s) => s.networkMs)).toEqual([99, 198]);
  });

  test("marks which reading each sample belongs to", () => {
    const w = new LatencyWindow(50);
    w.add(...engineCall(100, 2_000, 1_000));
    w.add(900, 762_000, 0);

    expect(w.series().map((s) => s.engineMs)).toEqual([1, null]);
    expect(w.series().map((s) => s.historyMs)).toEqual([null, 762]);
  });

  test("is capped by the window, like the percentiles are", () => {
    const w = new LatencyWindow(3);
    for (const ms of [10, 20, 30, 40]) w.add(ms, 1_000, 500);

    expect(w.series()).toHaveLength(3);
    expect(w.series()[0]!.networkMs).toBe(19);
  });

  test("is empty until a sample carries a server number", () => {
    const w = new LatencyWindow(50);
    w.add(120, null, null);
    expect(w.series()).toEqual([]);
  });

  test("cannot be mutated by its caller", () => {
    const w = new LatencyWindow(50);
    w.add(...engineCall(100, 1_000, 500));

    w.series().push({ engineMs: 999, historyMs: null, networkMs: 999 });

    expect(w.series()).toHaveLength(1);
  });
});
