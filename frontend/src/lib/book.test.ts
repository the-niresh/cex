import { describe, expect, it } from "vitest";
import { DepthBook } from "./book";
import type { DepthSnapshot, DepthUpdate } from "./types";

const SYM = "BTC_USDT";

function snapshot(depth_seq: bigint): DepthSnapshot {
  return {
    symbol: SYM,
    depth_seq,
    bids: [
      [50_142_000_000n, 8_000_000n],
      [50_141_000_000n, 1_250_000n],
    ],
    asks: [
      [50_143_000_000n, 4_210_000n],
      [50_144_000_000n, 850_000n],
    ],
  };
}

function update(depth_seq: bigint, deltas: DepthUpdate["deltas"]): DepthUpdate {
  return { symbol: SYM, depth_seq, deltas };
}

describe("applying deltas", () => {
  it("takes the next delta in sequence", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));

    const result = book.apply(update(11n, [{ side: "BUY", price: 50_142_000_000n, qty: 9_000_000n }]));

    expect(result).toBe("applied");
    expect(book.depthSeq).toBe(11n);
    expect(book.bids()[0]).toEqual({ price: 50_142_000_000n, qty: 9_000_000n, total: 9_000_000n });
  });

  it("adds a level that was not in the book", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));

    book.apply(update(11n, [{ side: "BUY", price: 50_142_500_000n, qty: 500_000n }]));

    expect(book.bids()[0]).toEqual({ price: 50_142_500_000n, qty: 500_000n, total: 500_000n });
  });

  it("treats a zero quantity as removing the level, not as a level of size zero", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));

    book.apply(update(11n, [{ side: "BUY", price: 50_142_000_000n, qty: 0n }]));

    expect(book.bids().map((l) => l.price)).toEqual([50_141_000_000n]);
    expect(book.bids().some((l) => l.qty === 0n)).toBe(false);
  });
});

describe("ordering", () => {
  it("returns bids best first, descending", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));
    book.apply(update(11n, [{ side: "BUY", price: 50_130_000_000n, qty: 1n }]));

    const prices = book.bids().map((l) => l.price);
    expect(prices).toEqual([...prices].sort((a, b) => (a < b ? 1 : -1)));
  });

  it("returns asks best first, ascending", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));
    book.apply(update(11n, [{ side: "SELL", price: 50_150_000_000n, qty: 1n }]));

    const prices = book.asks().map((l) => l.price);
    expect(prices).toEqual([...prices].sort((a, b) => (a < b ? -1 : 1)));
  });

  it("accumulates a running total down each side", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));

    expect(book.bids().map((l) => l.total)).toEqual([8_000_000n, 9_250_000n]);
  });
});

// ── the one that matters ──────────────────────────────────────────────
//
// `depth_seq` is monotonic per symbol. A jump means an update was missed, and
// applying the next delta on top of a book that is already wrong produces a
// book that stays wrong for as long as the connection lives — without ever
// looking wrong. The server does not announce it. The gap *is* the signal.

describe("resyncing on a gap", () => {
  it("reports a gap when the sequence jumps", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));

    const result = book.apply(update(12n, [{ side: "BUY", price: 50_142_000_000n, qty: 9_000_000n }]));

    expect(result).toBe("gap");
  });

  it("does not apply the delta that revealed the gap", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));
    const before = book.bids();

    book.apply(update(12n, [{ side: "BUY", price: 50_142_000_000n, qty: 9_000_000n }]));

    expect(book.bids()).toEqual(before);
    // The sequence must not advance past the hole either.
    expect(book.depthSeq).toBe(10n);
  });

  it("stays stale until a fresh snapshot arrives", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));

    book.apply(update(12n, []));
    expect(book.stale).toBe(true);

    // Every delta after the gap is refused too: the book cannot be trusted
    // again just because the numbers started lining up.
    expect(book.apply(update(13n, []))).toBe("gap");
  });

  it("refuses even a delta that lines up, once a gap has been seen", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));
    const before = book.bids();

    // 12 arrived, so 11 was missed. The sequence has not advanced past the
    // hole, which means the *next* number the book is waiting for is 11 —
    // and 11 turning up now must still not be applied. Accepting it would
    // leave the book permanently missing whatever 12 carried, because 12 has
    // already gone by. Only a snapshot can settle this.
    book.apply(update(12n, [{ side: "BUY", price: 50_142_000_000n, qty: 7n }]));

    expect(book.apply(update(11n, [{ side: "BUY", price: 50_142_000_000n, qty: 3n }]))).toBe("gap");
    expect(book.bids()).toEqual(before);
  });

  it("recovers once the snapshot is refetched", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));
    book.apply(update(12n, []));

    book.reset(snapshot(20n));

    expect(book.stale).toBe(false);
    expect(book.apply(update(21n, [{ side: "SELL", price: 50_143_000_000n, qty: 1n }]))).toBe("applied");
    expect(book.asks()[0]).toEqual({ price: 50_143_000_000n, qty: 1n, total: 1n });
  });

  it("ignores a replayed delta instead of applying it twice", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));
    book.apply(update(11n, [{ side: "BUY", price: 50_142_000_000n, qty: 9_000_000n }]));

    // Recovery republishes events, so seeing one again is routine — and is
    // emphatically not a gap.
    const result = book.apply(update(11n, [{ side: "BUY", price: 50_142_000_000n, qty: 1n }]));

    expect(result).toBe("stale-update");
    expect(book.stale).toBe(false);
    expect(book.bids()[0].qty).toBe(9_000_000n);
  });

  it("refuses a delta before any snapshot has been loaded", () => {
    const book = new DepthBook();

    expect(book.apply(update(1n, []))).toBe("gap");
  });
});

describe("the spread", () => {
  it("is the distance between the best bid and the best ask", () => {
    const book = new DepthBook();
    book.reset(snapshot(10n));

    expect(book.spread()).toBe(1_000_000n);
  });

  it("is unknown when one side is empty", () => {
    const book = new DepthBook();
    book.reset({ symbol: SYM, depth_seq: 1n, bids: [], asks: [] });

    expect(book.spread()).toBeNull();
  });
});

describe("a snapshot that is older than the book", () => {
  // The bug this guards: the read path can lag the event path. The engine
  // answers `/depth` between blocking stream reads, so a resync racing a
  // freshly published delta gets back a view from *before* it. Rebuilding
  // from that rolls the book backwards, deleting a level the exchange really
  // has — and because nothing further changes, no later delta ever puts it
  // back. The screen sits there wrong until someone reloads.
  it("is refused rather than rolling the book backwards", () => {
    const book = new DepthBook();
    book.reset(snapshot(9n));

    const newLevel = 31_111_110_000n;
    expect(book.apply(update(10n, [{ side: "BUY", price: newLevel, qty: 1_000_000n }]))).toBe(
      "applied",
    );
    expect(book.bids().some((l) => l.price === newLevel)).toBe(true);

    // The lagging snapshot arrives, still describing the world at seq 9.
    const accepted = book.reset(snapshot(9n));

    expect(accepted).toBe(false);
    expect(book.depthSeq).toBe(10n);
    expect(
      book.bids().some((l) => l.price === newLevel),
      "the applied delta must survive a stale snapshot",
    ).toBe(true);
  });

  it("still accepts a snapshot at or ahead of the book", () => {
    const book = new DepthBook();
    book.reset(snapshot(9n));

    expect(book.reset(snapshot(9n))).toBe(true);
    expect(book.reset(snapshot(12n))).toBe(true);
    expect(book.depthSeq).toBe(12n);
  });

  it("accepts any snapshot when there is no book yet", () => {
    const book = new DepthBook();
    expect(book.reset(snapshot(3n))).toBe(true);
    expect(book.depthSeq).toBe(3n);
  });
});
