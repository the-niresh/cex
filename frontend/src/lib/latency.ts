// Three numbers, never summed, because they are three different subsystems.
//
// `engine` is time inside the matching engine (`x-cex-engine-us`). `history` is
// a request the engine never saw — `/trades` and `/candles` are served from
// Postgres and report an engine time of exactly zero. `network` is what the
// wire added, and depends on where the reader is sitting rather than on the
// exchange at all.
//
// Averaging the first two together is what this split exists to prevent: two
// history queries against a cloud database will bury eighteen sub-2ms engine
// calls, and the screen ends up blaming the matching engine for a round trip
// to us-east-1.

// The p99 of `x-cex-engine-us` from the localhost load run — the same quantity
// the engine reading shows, so the comparison is like for like. A live p50
// above the measured p99 means the exchange is behaving worse than it did in
// 99% of samples, which is the only non-arbitrary definition of "degraded"
// available.
//
// Re-derive with, against a freshly reset stack:
//   cargo run -p cex-loadgen -- --host http://localhost:8080 \
//     --ws ws://localhost:8081 --count 2000
// Update this whenever that run is repeated, and update docs/internals.md too.
export const ENGINE_P99_BASELINE_MS = 8.4;

/** One measurement of one subsystem, over the window. */
export interface Reading {
  p50: number | null;
  p99: number | null;
  count: number;
}

export interface LatencyStats {
  engine: Reading;
  history: Reading;
  network: Reading;
  /**
   * Responses seen while no reading has a single sample — i.e. the exchange
   * answered, but without the timing headers this strip is made of. Non-zero
   * means the deployed API predates `crates/api/src/timing.rs`, or a proxy in
   * front of it is dropping the headers.
   */
  unreported: number;
}

export interface Sample {
  /** Set when the request reached the engine; null when it was served from the database. */
  engineMs: number | null;
  /** Set when the request never reached the engine. Exactly one of the two is non-null. */
  historyMs: number | null;
  networkMs: number;
}

const EMPTY: Reading = { p50: null, p99: null, count: 0 };

const percentile = (sorted: number[], q: number): number =>
  sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]!;

function reading(values: number[], places: number): Reading {
  if (values.length === 0) return EMPTY;
  const sorted = [...values].sort((a, b) => a - b);
  const round = (v: number) => Math.round(v * 10 ** places) / 10 ** places;
  return {
    p50: round(percentile(sorted, 0.5)),
    p99: round(percentile(sorted, 0.99)),
    count: values.length,
  };
}

export class LatencyWindow {
  private readonly samples: Sample[] = [];

  /**
   * Responses that arrived without usable timing headers.
   *
   * ⚠️ Without this the strip cannot tell "nobody has made a request yet" from
   * "the server is not reporting", and renders an identical row of dashes for
   * both. That is exactly what a deployed API built before `timing.rs` looks
   * like, and it took a `curl -D -` to find out rather than a glance at the
   * screen the number is printed on.
   */
  private unreported = 0;

  constructor(private readonly capacity = 50) {}

  /**
   * `serverUs` is `x-cex-server-us` and `engineUs` is `x-cex-engine-us`, or
   * null when either header was absent.
   *
   * Both are required. A missing measurement is not a fast one — rendering it
   * as zero would advertise an exchange faster than physics — and a server
   * time without an engine time cannot be filed under either reading, because
   * "did this reach the engine" is exactly what the engine header answers.
   */
  add(totalMs: number, serverUs: number | null, engineUs: number | null): void {
    if (serverUs === null || !Number.isFinite(serverUs)) {
      this.unreported += 1;
      return;
    }
    if (engineUs === null || !Number.isFinite(engineUs)) {
      this.unreported += 1;
      return;
    }

    const serverMs = serverUs / 1000;
    // Zero is meaningful, not missing: the middleware sets the header on every
    // response, so an engine time of exactly zero means the request was served
    // without the engine ever being asked.
    const reachedEngine = engineUs > 0;

    this.samples.push({
      engineMs: reachedEngine ? engineUs / 1000 : null,
      historyMs: reachedEngine ? null : serverMs,
      // Clamped, because performance.now() and the server's own clock are not
      // the same clock and a fast local call can invert them.
      networkMs: Math.max(0, totalMs - serverMs),
    });

    if (this.samples.length > this.capacity) this.samples.shift();
  }

  /**
   * The samples themselves, oldest first — what a sparkline needs, since it
   * reads left to right and so does time.
   *
   * A copy, not the live array: handing out the internal one would let a
   * caller mutate the window, and a chart holding the reference would see it
   * shift under itself between renders.
   */
  series(): Sample[] {
    return this.samples.map((s) => ({ ...s }));
  }

  stats(): LatencyStats {
    const engine = this.samples.map((s) => s.engineMs).filter((v) => v !== null);
    const history = this.samples.map((s) => s.historyMs).filter((v) => v !== null);
    const network = this.samples.map((s) => s.networkMs);

    return {
      // Engine time runs in the low milliseconds, so it keeps a decimal. The
      // other two are tens to hundreds, where a decimal is false precision.
      engine: reading(engine, 1),
      history: reading(history, 0),
      network: reading(network, 0),
      // Only interesting while nothing has been measured. Once a single sample
      // lands, the server is plainly reporting and the count is noise.
      unreported: this.samples.length === 0 ? this.unreported : 0,
    };
  }
}
