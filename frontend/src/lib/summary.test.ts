import { describe, expect, it } from "vitest";
import { feesPaid, valueLocked } from "./summary";
import type { Market, MyFill, Order } from "./types";

const BTC_USDT: Market = {
  symbol: "BTC_USDT",
  base: "BTC",
  quote: "USDT",
  base_decimals: 8n,
  quote_decimals: 6n,
  tick_size: 10_000n,
  lot_size: 1_000n,
  min_notional: 1_000_000n,
  maker_fee_bps: 0n,
  taker_fee_bps: 5n,
};

const ETH_USDT: Market = {
  ...BTC_USDT,
  symbol: "ETH_USDT",
  base: "ETH",
  tick_size: 1_000n,
};

const order = (over: Partial<Order>): Order => ({
  order_id: 1n,
  user_id: "u",
  symbol: "BTC_USDT",
  side: "BUY",
  order_type: "LIMIT",
  price: 50_000_000_000n,
  qty: 100_000_000n,
  filled_qty: 0n,
  status: "OPEN",
  ...over,
});

const fill = (over: Partial<MyFill>): MyFill =>
  ({
    seq: 1n,
    idx: 0n,
    symbol: "BTC_USDT",
    order_id: 1n,
    side: "BUY",
    role: "TAKER",
    price: 50_000_000_000n,
    qty: 100_000_000n,
    notional: 50_000_000_000n,
    fee: 50_000n,
    timestamp_ms: 0n,
    ...over,
  }) as MyFill;

describe("valueLocked", () => {
  it("is empty when nothing is resting", () => {
    expect(valueLocked([], [BTC_USDT])).toBe("");
  });

  it("holds quote for a bid and base for an ask", () => {
    const text = valueLocked(
      [order({ side: "BUY" }), order({ order_id: 2n, side: "SELL" })],
      [BTC_USDT],
    );
    // One whole BTC bid at 50,000 holds 50,000 USDT; the ask holds the coin.
    expect(text).toBe("50,000.00 USDT + 1.00 BTC");
  });

  it("counts only the unfilled remainder", () => {
    const text = valueLocked([order({ qty: 100_000_000n, filled_qty: 75_000_000n })], [BTC_USDT]);
    expect(text).toBe("12,500.00 USDT");
  });

  it("adds up across markets that share an asset", () => {
    const text = valueLocked(
      [order({ side: "BUY" }), order({ order_id: 2n, symbol: "ETH_USDT", side: "BUY" })],
      [BTC_USDT, ETH_USDT],
    );
    expect(text).toBe("100,000.00 USDT");
  });

  it("skips a market order, which holds nothing at a price", () => {
    expect(valueLocked([order({ price: null })], [BTC_USDT])).toBe("");
  });

  it("skips an order whose market is unknown", () => {
    expect(valueLocked([order({ symbol: "DOGE_USDT" })], [BTC_USDT])).toBe("");
  });
});

describe("feesPaid", () => {
  it("is empty with no fills", () => {
    expect(feesPaid([], [BTC_USDT])).toBe("");
  });

  it("groups by the asset actually charged, never summing across them", () => {
    // A buy is charged in what it received (base); a sell in quote. Adding BTC
    // to USDT because both came from BTC_USDT would be a number meaning nothing.
    const text = feesPaid([fill({ side: "BUY" }), fill({ idx: 1n, side: "SELL" })], [BTC_USDT]);
    // Padded to each asset's own precision, which is what the fee line has
    // always done — the point is that they are never added together.
    expect(text).toBe("0.00050000 BTC + 0.050000 USDT");
  });

  it("reports at the asset's own scale, not a fixed two places", () => {
    // Rounded to 2dp this prints "0.00" — a summary that says you paid nothing.
    expect(feesPaid([fill({ side: "BUY", fee: 500n })], [BTC_USDT])).toBe("0.00000500 BTC");
  });
});
