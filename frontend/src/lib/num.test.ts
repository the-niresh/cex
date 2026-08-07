import { describe, expect, test } from "vitest";

import { feeAsset, formatAtoms } from "./num";
import type { Market } from "./types";

// BTC_USDT as the API reports it: BTC has 8 decimals, USDT has 6. The two
// scales differing is the whole point of these tests — reading a fee at the
// wrong one is off by a factor of 100 and still looks like a plausible number.
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

describe("feeAsset", () => {
  test("a buyer pays in the base asset, at base scale", () => {
    expect(feeAsset("BUY", BTC_USDT)).toEqual({ asset: "BTC", decimals: 8n });
  });

  test("a seller pays in the quote asset, at quote scale", () => {
    expect(feeAsset("SELL", BTC_USDT)).toEqual({ asset: "USDT", decimals: 6n });
  });

  // The regression this function exists to stop: a buy-side taker fee of 100
  // BTC atoms is 0.000001 BTC. Rendered at USDT's 6 decimals it reads
  // 0.000100 — a hundred times larger, and labelled as the wrong asset.
  test("renders a buy-side fee at its own scale, not the quote's", () => {
    const { asset, decimals } = feeAsset("BUY", BTC_USDT);
    expect(formatAtoms(100n, decimals)).toBe("0.00000100");
    expect(asset).toBe("BTC");
    expect(formatAtoms(100n, BTC_USDT.quote_decimals)).toBe("0.000100");
  });
});
