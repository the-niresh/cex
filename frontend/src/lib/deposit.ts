import type { Market } from "./types";

/** Preset deposit amounts in whole units for USDT. */
export const USDT_PRESETS = [1_000, 10_000, 100_000] as const;

export const DEPOSIT_LIMITS: Record<string, number> = {
  USDT: 1_000_000,
  BTC: 10,
  ETH: 100,
  SOL: 10_000,
};

/** Assets that can be deposited, in display order. */
export function depositAssets(markets: Market[]): string[] {
  return [...new Set(markets.flatMap((m) => [m.quote, m.base]))];
}

export function decimalsForAsset(asset: string, markets: Market[]): bigint {
  for (const market of markets) {
    if (market.base === asset) return market.base_decimals;
    if (market.quote === asset) return market.quote_decimals;
  }
  return 8n;
}

/** Label for the deposit dialog title. */
export function depositAssetLabel(asset: string): string {
  if (asset === "USDT") return "USDT (demo dollars)";
  return asset;
}
