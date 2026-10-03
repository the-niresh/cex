import { describe, expect, it } from "vitest";
import { buildHoldingsValueSeries } from "../components/PortfolioValueChart";
import type { Balance, Candle, Market } from "./types";

const markets: Market[] = [
  {
    symbol: "BTC_USDT",
    base: "BTC",
    quote: "USDT",
    base_decimals: 8n,
    quote_decimals: 6n,
    tick_size: 10_000n,
    lot_size: 1_000n,
    min_notional: 1_000_000n,
    maker_fee_bps: 2n,
    taker_fee_bps: 5n,
  },
];

describe("buildHoldingsValueSeries", () => {
  it("returns a flat zero line with no holdings", () => {
    const points = buildHoldingsValueSeries([], new Map(), markets);
    expect(points.length).toBe(24);
    expect(points.every((p) => p.value === 0)).toBe(true);
  });

  it("values USDT holdings at face value across buckets", () => {
    const balances: Balance[] = [{ asset: "USDT", available: 10_000_000_000n, locked: 0n }];
    const candles: Candle[] = [
      {
        time_ms: 1_000_000n,
        open: 50_000_000_000n,
        high: 50_000_000_000n,
        low: 50_000_000_000n,
        close: 50_000_000_000n,
        volume: 0n,
        trades: 0n,
      },
    ];
    const candlesBySymbol = new Map<string, Candle[]>([["BTC_USDT", candles]]);
    const points = buildHoldingsValueSeries(balances, candlesBySymbol, markets);
    expect(points.every((p) => p.value === 10_000)).toBe(true);
  });

  it("draws a flat series at 10,000 for 10,000 USDT without candle history", () => {
    const balances: Balance[] = [{ asset: "USDT", available: 10_000_000_000n, locked: 0n }];
    const points = buildHoldingsValueSeries(balances, new Map(), markets);
    expect(points.length).toBe(24);
    expect(points.every((p) => p.value === 10_000)).toBe(true);
  });
});
