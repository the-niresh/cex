/**
 * Portfolio valuation in quote (USDT) atoms.
 *
 * USDT is worth face value. Other assets need a last trade price in quote
 * atoms per whole base unit. Assets with no price are excluded from the total.
 */

import { notional } from "./num";
import type { Balance, Market } from "./types";

export interface PortfolioRow {
  asset: string;
  available: bigint;
  locked: bigint;
  /** Quote atoms per one whole unit, or null when unknown. */
  lastPrice: bigint | null;
  /** Value in USDT atoms, or null when there is no price. */
  value: bigint | null;
  /** Share of the counted total, 0-100, or null when not in the total. */
  sharePct: number | null;
}

export interface PortfolioSummary {
  rows: PortfolioRow[];
  /** Sum of row values that had a price. */
  total: bigint;
  /** Assets left out because no price was available. */
  withoutPrice: string[];
}

/** USDT atoms for one whole USDT. */
const USDT_UNIT = 1_000_000n;

/**
 * Value one asset balance in USDT atoms.
 *
 * `price` is quote atoms per whole base unit. For USDT pass `USDT_UNIT`.
 */
export function assetValue(
  amount: bigint,
  asset: string,
  price: bigint | null,
  markets: Market[],
): bigint | null {
  if (price === null) return null;
  if (asset === "USDT") return amount;
  const market = markets.find((m) => m.base === asset);
  if (!market) return null;
  return notional(price, amount, market.base_decimals, "down");
}

/** Build portfolio rows and total from balances and last prices. */
export function summarizePortfolio(
  balances: Balance[],
  prices: Map<string, bigint>,
  markets: Market[],
): PortfolioSummary {
  const withoutPrice: string[] = [];
  const rows: PortfolioRow[] = balances.map((b) => {
    const amount = b.available + b.locked;
    const price = prices.get(b.asset) ?? (b.asset === "USDT" ? USDT_UNIT : null);
    const value = assetValue(amount, b.asset, price ?? null, markets);
    if (value === null && amount > 0n) withoutPrice.push(b.asset);
    return {
      asset: b.asset,
      available: b.available,
      locked: b.locked,
      lastPrice: price ?? null,
      value,
      sharePct: null,
    };
  });

  let total = 0n;
  for (const row of rows) {
    if (row.value !== null) total += row.value;
  }

  for (const row of rows) {
    if (row.value === null || total === 0n) continue;
    row.sharePct = Number((row.value * 10_000n) / total) / 100;
  }

  return { rows, total, withoutPrice };
}
