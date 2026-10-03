import { describe, expect, it } from "vitest";
import { assetValue, summarizePortfolio } from "./portfolio";
import type { Balance, Market } from "./types";

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

describe("summarizePortfolio", () => {
  it("total equals the sum of priced asset values", () => {
    const balances: Balance[] = [
      { asset: "USDT", available: 5_000_000_000n, locked: 0n },
      { asset: "BTC", available: 100_000_000n, locked: 0n },
    ];
    const prices = new Map<string, bigint>([
      ["USDT", 1_000_000n],
      ["BTC", 50_000_000_000n],
    ]);
    const { rows, total } = summarizePortfolio(balances, prices, markets);
    const sum = rows.reduce((acc, r) => acc + (r.value ?? 0n), 0n);
    expect(total).toBe(sum);
    expect(total).toBe(5_000_000_000n + 50_000_000_000n);
  });

  it("excludes an asset with no price from the total", () => {
    const balances: Balance[] = [
      { asset: "USDT", available: 1_000_000n, locked: 0n },
      { asset: "BTC", available: 100_000_000n, locked: 0n },
    ];
    const prices = new Map<string, bigint>([["USDT", 1_000_000n]]);
    const { total, withoutPrice, rows } = summarizePortfolio(balances, prices, markets);
    expect(total).toBe(1_000_000n);
    expect(withoutPrice).toContain("BTC");
    expect(rows.find((r) => r.asset === "BTC")?.value).toBeNull();
  });
});

describe("assetValue", () => {
  it("values USDT at face value", () => {
    expect(assetValue(2_500_000n, "USDT", 1_000_000n, markets)).toBe(2_500_000n);
  });
});
