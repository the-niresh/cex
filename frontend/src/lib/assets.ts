/** Distinct badge colour per asset, from theme tokens only. */
const KNOWN: Record<string, string> = {
  BTC: "var(--color-asset-btc)",
  ETH: "var(--color-asset-eth)",
  SOL: "var(--color-asset-sol)",
  USDT: "var(--color-asset-usdt)",
};

const FALLBACK = [
  "var(--color-asset-1)",
  "var(--color-asset-2)",
  "var(--color-asset-3)",
];

export function assetColor(asset: string): string {
  const known = KNOWN[asset];
  if (known) return known;
  let hash = 0;
  for (let i = 0; i < asset.length; i++) hash = (hash * 31 + asset.charCodeAt(i)) | 0;
  return FALLBACK[Math.abs(hash) % FALLBACK.length];
}

/** First letter shown in a round coin badge. */
export function assetInitial(asset: string): string {
  return asset.charAt(0);
}
