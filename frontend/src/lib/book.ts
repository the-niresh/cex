import type { DepthSnapshot, DepthUpdate, Side } from "./types";

/** A rung of the ladder, with the running total down that side. */
export interface Level {
  price: bigint;
  qty: bigint;
  total: bigint;
}

/**
 * What happened to a delta.
 *
 * `gap` is the one that matters: it means an update was missed and the caller
 * must refetch `GET /depth/:symbol` rather than carry on.
 */
export type ApplyResult = "applied" | "gap" | "stale-update";

/**
 * A live order book, rebuilt from a REST snapshot and kept current by deltas.
 *
 * ## The gap rule
 *
 * `depth_seq` is monotonic per symbol. Every delta must be exactly one past the
 * one before it. If it is not, an update was lost somewhere — a dropped frame,
 * a reconnect, a subscriber that fell behind — and the book in memory is
 * already wrong.
 *
 * Applying the next delta on top of a wrong book gives a book that stays wrong
 * for the life of the connection and *never looks wrong*: the prices are
 * plausible, the sizes are plausible, and a trader sizes an order against
 * liquidity that is not there. So a gap is refused, the book is marked stale,
 * and nothing is applied again until a fresh snapshot arrives.
 *
 * The server does not announce any of this. The gap is the signal.
 */
export class DepthBook {
  private bidLevels = new Map<bigint, bigint>();
  private askLevels = new Map<bigint, bigint>();

  /** The sequence of the last update folded in, or null before the snapshot. */
  depthSeq: bigint | null = null;

  /** True once a gap was seen, until {@link reset} brings a new snapshot. */
  stale = false;

  symbol: string | null = null;

  /** Rebuild from a REST snapshot. The only way out of a stale book. */
  reset(snapshot: DepthSnapshot): void {
    this.symbol = snapshot.symbol;
    this.bidLevels = new Map(snapshot.bids.map(([price, qty]) => [price, qty]));
    this.askLevels = new Map(snapshot.asks.map(([price, qty]) => [price, qty]));
    this.depthSeq = snapshot.depth_seq;
    this.stale = false;
  }

  apply(update: DepthUpdate): ApplyResult {
    // Nothing to apply a delta *to* yet. Same remedy as a gap: fetch first.
    if (this.depthSeq === null) return "gap";

    // Already stale — a later sequence lining up again does not mean the hole
    // in the middle healed.
    if (this.stale) return "gap";

    // The engine republishes events after recovery, so seeing one twice is
    // routine. Re-applying it would double-count; it is not a gap.
    if (update.depth_seq <= this.depthSeq) return "stale-update";

    if (update.depth_seq !== this.depthSeq + 1n) {
      this.stale = true;
      return "gap";
    }

    for (const delta of update.deltas) {
      const side = delta.side === "BUY" ? this.bidLevels : this.askLevels;
      // A zero quantity removes the price level. It does not mean a level that
      // is present and empty — one of those would sit in the ladder forever.
      if (delta.qty === 0n) side.delete(delta.price);
      else side.set(delta.price, delta.qty);
    }

    this.depthSeq = update.depth_seq;
    return "applied";
  }

  bids(limit?: number): Level[] {
    return this.sorted(this.bidLevels, "BUY", limit);
  }

  asks(limit?: number): Level[] {
    return this.sorted(this.askLevels, "SELL", limit);
  }

  /** Best ask minus best bid, in quote atoms. Null while a side is empty. */
  spread(): bigint | null {
    const bid = this.bids(1)[0];
    const ask = this.asks(1)[0];
    if (!bid || !ask) return null;
    return ask.price - bid.price;
  }

  /** The midpoint, for a market order's expected cost. Null while a side is empty. */
  mid(): bigint | null {
    const bid = this.bids(1)[0];
    const ask = this.asks(1)[0];
    if (!bid || !ask) return null;
    return (bid.price + ask.price) / 2n;
  }

  private sorted(levels: Map<bigint, bigint>, side: Side, limit?: number): Level[] {
    const descending = side === "BUY";
    const prices = [...levels.keys()].sort((a, b) => {
      if (a === b) return 0;
      const aFirst = descending ? a > b : a < b;
      return aFirst ? -1 : 1;
    });

    const wanted = limit === undefined ? prices : prices.slice(0, limit);

    let total = 0n;
    return wanted.map((price) => {
      const qty = levels.get(price) as bigint;
      total += qty;
      return { price, qty, total };
    });
  }
}
