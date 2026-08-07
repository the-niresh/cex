import { describe, expect, test } from "vitest";

import { liveFillFrom, mergeFills } from "./fills";
import type { Market, MyFill, OrderUpdate, Role, Side } from "./types";

const BTC_USDT: Market = {
  symbol: "BTC_USDT",
  base: "BTC",
  quote: "USDT",
  base_decimals: 8n,
  quote_decimals: 6n,
  tick_size: 1000000n,
  lot_size: 1000n,
  min_notional: 1000000n,
  maker_fee_bps: 1n,
  taker_fee_bps: 5n,
};

const fillEvent: OrderUpdate = {
  event: "fill",
  order_id: 191n,
  symbol: "BTC_USDT",
  side: "BUY",
  price: 50119500000n,
  qty: 200000n,
  fee: 100n,
  role: "TAKER",
  idx: 0n,
};

const historyRow = (seq: bigint, idx: bigint): MyFill => ({
  seq,
  idx,
  symbol: "BTC_USDT",
  order_id: 191n,
  side: "BUY",
  role: "TAKER",
  price: 50119500000n,
  qty: 200000n,
  notional: 100239000n,
  fee: 100n,
  timestamp_ms: 1786111424286n,
});

describe("liveFillFrom", () => {
  test("carries the fill through and derives the notional", () => {
    const built = liveFillFrom(fillEvent, 262n, [BTC_USDT]);
    expect(built).not.toBeNull();
    expect(built).toMatchObject({
      seq: 262n,
      idx: 0n,
      order_id: 191n,
      side: "BUY",
      price: 50119500000n,
      qty: 200000n,
      fee: 100n,
      role: "TAKER",
      // price × qty / 10^8, the same arithmetic the API reports.
      notional: 100239000n,
    });
  });

  // The feed serialises Role as snake_case and `/orders/history` as upper —
  // the same concept in two casings. Normalising here is what stops a live
  // maker fill rendering as a taker until the history row replaces it.
  test("normalises the role the feed sends to the one the API reports", () => {
    expect(liveFillFrom({ ...fillEvent, role: "maker" as Role }, 1n, [BTC_USDT])?.role).toBe(
      "MAKER",
    );
    expect(liveFillFrom({ ...fillEvent, role: "taker" as Role }, 1n, [BTC_USDT])?.role).toBe(
      "TAKER",
    );
    expect(liveFillFrom({ ...fillEvent, side: "sell" as Side }, 1n, [BTC_USDT])?.side).toBe("SELL");
  });

  test("ignores events that are not fills", () => {
    expect(liveFillFrom({ event: "accepted", order_id: 1n }, 1n, [BTC_USDT])).toBeNull();
  });

  // Better an absent row than one carrying a notional computed against a
  // guessed scale.
  test("refuses to build one for an unknown market", () => {
    expect(liveFillFrom(fillEvent, 262n, [])).toBeNull();
  });
});

describe("mergeFills", () => {
  test("shows a live fill the history has not caught up to", () => {
    const live = liveFillFrom(fillEvent, 262n, [BTC_USDT])!;
    expect(mergeFills([], [live], 60)).toEqual([live]);
  });

  test("drops the live copy once the history row exists", () => {
    const live = liveFillFrom(fillEvent, 262n, [BTC_USDT])!;
    const merged = mergeFills([historyRow(262n, 0n)], [live], 60);
    expect(merged).toHaveLength(1);
    expect(merged[0].timestamp_ms).toBe(1786111424286n);
  });

  // One command can produce several fills, so seq alone would collapse them.
  test("keeps fills that share a seq but differ by idx", () => {
    const live = liveFillFrom({ ...fillEvent, idx: 1n }, 262n, [BTC_USDT])!;
    expect(mergeFills([historyRow(262n, 0n)], [live], 60)).toHaveLength(2);
  });

  test("caps the merged list", () => {
    const history = [historyRow(1n, 0n), historyRow(2n, 0n), historyRow(3n, 0n)];
    expect(mergeFills(history, [], 2)).toHaveLength(2);
  });
});
