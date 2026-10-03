import type { Market } from "./types";

/** Read `?market=` from the location, falling back to the first listed market. */
export function marketFromParam(requested: string | null, markets: Market[]): string {
  if (requested && markets.some((m) => m.symbol === requested)) {
    return requested;
  }
  return markets[0]?.symbol ?? "BTC_USDT";
}

/** Trade screen link for a market tab. */
export function tradeHref(symbol: string): string {
  return `/?market=${encodeURIComponent(symbol)}`;
}
