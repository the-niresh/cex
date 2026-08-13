// Two numbers, never one. The API is on a different host to this page, so a
// single round-trip figure is mostly a measure of where the reader is sitting.
// Reporting the exchange's own time separately is the only honest version.

// The p99 of `x-cex-server-us` from the localhost load run: the same quantity
// this screen displays as `engine`, so the comparison is like for like. A live
// p50 above the measured p99 means the exchange is behaving worse than it did
// in 99% of samples, which is the only non-arbitrary definition of "degraded"
// available. Deliberately not the p99 of `x-cex-engine-us`, which is a
// strictly smaller number measuring a strictly smaller thing.
//
// Re-derive with, against a freshly reset stack:
//   cargo run -p cex-loadgen -- --host http://localhost:8080 \
//     --ws ws://localhost:8081 --count 2000
// Update this whenever that run is repeated, and update docs/internals.md too.
export const ENGINE_P99_BASELINE_MS = 8.6;

export interface LatencyStats {
  engineP50: number | null;
  engineP99: number | null;
  networkP50: number | null;
  networkP99: number | null;
  count: number;
}

interface Sample {
  engineMs: number;
  networkMs: number;
}

const percentile = (sorted: number[], q: number): number =>
  sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]!;

export class LatencyWindow {
  private readonly samples: Sample[] = [];

  constructor(private readonly capacity = 50) {}

  /** `serverUs` is `x-cex-server-us`, or null when the header was absent. */
  add(totalMs: number, serverUs: number | null): void {
    // A missing header is a missing measurement. Treating it as zero would
    // render the exchange faster than it is, which is the one failure mode
    // this whole readout exists to avoid.
    if (serverUs === null || !Number.isFinite(serverUs)) return;

    const engineMs = serverUs / 1000;
    this.samples.push({
      engineMs,
      // Clamped, because performance.now() and the server's own clock are not
      // the same clock and a fast local call can invert them.
      networkMs: Math.max(0, totalMs - engineMs),
    });

    if (this.samples.length > this.capacity) this.samples.shift();
  }

  stats(): LatencyStats {
    if (this.samples.length === 0) {
      return {
        engineP50: null,
        engineP99: null,
        networkP50: null,
        networkP99: null,
        count: 0,
      };
    }

    const engine = this.samples.map((s) => s.engineMs).sort((a, b) => a - b);
    const network = this.samples.map((s) => s.networkMs).sort((a, b) => a - b);

    return {
      engineP50: Math.round(percentile(engine, 0.5) * 10) / 10,
      engineP99: Math.round(percentile(engine, 0.99) * 10) / 10,
      networkP50: Math.round(percentile(network, 0.5)),
      networkP99: Math.round(percentile(network, 0.99)),
      count: this.samples.length,
    };
  }
}
