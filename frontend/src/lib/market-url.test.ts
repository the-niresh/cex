import { describe, expect, it } from "vitest";
import { marketFromParam, tradeHref } from "./market-url";
import type { Market } from "./types";

const markets: Market[] = [
  {
    symbol: "BTC_USDT",
    base: "BTC",
    quote: "USDT",
    base_decimals: 8n,
    quote_decimals: 6n,
    tick_size: 100n,
    lot_size: 1n,
    min_notional: 1_000_000n,
    maker_fee_bps: 10n,
    taker_fee_bps: 20n,
  },
  {
    symbol: "ETH_USDT",
    base: "ETH",
    quote: "USDT",
    base_decimals: 8n,
    quote_decimals: 6n,
    tick_size: 10n,
    lot_size: 1n,
    min_notional: 1_000_000n,
    maker_fee_bps: 10n,
    taker_fee_bps: 20n,
  },
];

describe("marketFromParam", () => {
  it("uses a known market from the query string", () => {
    expect(marketFromParam("ETH_USDT", markets)).toBe("ETH_USDT");
  });

  it("falls back to the first market when the param is missing", () => {
    expect(marketFromParam(null, markets)).toBe("BTC_USDT");
  });

  it("falls back to the first market when the param is unknown", () => {
    expect(marketFromParam("DOGE_USDT", markets)).toBe("BTC_USDT");
  });
});

describe("tradeHref", () => {
  it("builds a shareable trade link", () => {
    expect(tradeHref("ETH_USDT")).toBe("/?market=ETH_USDT");
  });
});
