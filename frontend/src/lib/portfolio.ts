/**
 * Portfolio valuation in quote (USDT) atoms.
 *
 * USDT is worth face value. Other assets need a last trade price in quote
 * atoms per whole base unit. Assets with no price are excluded from the total.
 */

import { depositAssets } from "./deposit";
import { notional } from "./num";
import type { Balance, Market } from "./types";

export interface PortfolioRow {
  asset: string;
  available: bigint;
  locked: bigint;
  /** Available plus locked. */
  total: bigint;
  /** Quote atoms per one whole unit, or null when unknown. */
  lastPrice: bigint | null;
  /** Total value in USDT atoms, or null when there is no price. */
  value: bigint | null;
  /** Available balance in USDT atoms. */
  availableValue: bigint | null;
  /** Locked balance in USDT atoms. */
  lockedValue: bigint | null;
  /** Share of the counted total, 0-100, or null when not in the total. */
  sharePct: number | null;
}

export interface PortfolioSummary {
  rows: PortfolioRow[];
  /** Sum of row values that had a price. */
  total: bigint;
  /** Sum of available balances priced in USDT. */
  availableTotal: bigint;
  /** Sum of locked balances priced in USDT. */
  lockedTotal: bigint;
  /** Assets with a non-zero balance. */
  assetsHeld: number;
  /** Assets left out because no price was available. */
  withoutPrice: string[];
}

/** USDT atoms for one whole USDT. */
const USDT_UNIT = 1_000_000n;

/**
 * Every asset the exchange supports for deposit, in fixed display order.
 * Taken from markets rather than hard-coded.
 */
export function tradeableAssets(markets: Market[]): string[] {
  const raw = depositAssets(markets);
  // USDT first, then bases in stable order.
  const rest = raw.filter((a) => a !== "USDT").sort();
  return raw.includes("USDT") ? ["USDT", ...rest] : rest;
}

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

function priceFor(asset: string, prices: Map<string, bigint>): bigint | null {
  return prices.get(asset) ?? (asset === "USDT" ? USDT_UNIT : null);
}

/** Build portfolio rows and total from balances and last prices. */
export function summarizePortfolio(
  balances: Balance[],
  prices: Map<string, bigint>,
  markets: Market[],
): PortfolioSummary {
  const assets = tradeableAssets(markets);
  const byAsset = new Map(balances.map((b) => [b.asset, b]));
  const withoutPrice: string[] = [];

  const rows: PortfolioRow[] = assets.map((asset) => {
    const balance = byAsset.get(asset) ?? { asset, available: 0n, locked: 0n };
    const total = balance.available + balance.locked;
    const price = priceFor(asset, prices);
    const value = assetValue(total, asset, price, markets);
    const availableValue = assetValue(balance.available, asset, price, markets);
    const lockedValue = assetValue(balance.locked, asset, price, markets);
    if (value === null && total > 0n) withoutPrice.push(asset);
    return {
      asset,
      available: balance.available,
      locked: balance.locked,
      total,
      lastPrice: price,
      value,
      availableValue,
      lockedValue,
      sharePct: null,
    };
  });

  const fixedOrder = new Map(assets.map((a, i) => [a, i]));
  rows.sort((a, b) => {
    const aZero = a.total === 0n;
    const bZero = b.total === 0n;
    if (aZero !== bZero) return aZero ? 1 : -1;
    if (!aZero && !bZero) {
      const aVal = a.value ?? 0n;
      const bVal = b.value ?? 0n;
      if (aVal !== bVal) return aVal > bVal ? -1 : 1;
    }
    return (fixedOrder.get(a.asset) ?? 0) - (fixedOrder.get(b.asset) ?? 0);
  });

  let total = 0n;
  let availableTotal = 0n;
  let lockedTotal = 0n;
  let assetsHeld = 0;
  for (const row of rows) {
    if (row.total > 0n) assetsHeld += 1;
    if (row.value !== null) total += row.value;
    if (row.availableValue !== null) availableTotal += row.availableValue;
    if (row.lockedValue !== null) lockedTotal += row.lockedValue;
  }

  for (const row of rows) {
    if (row.value === null || total === 0n) continue;
    row.sharePct = Number((row.value * 10_000n) / total) / 100;
  }

  return { rows, total, availableTotal, lockedTotal, assetsHeld, withoutPrice };
}
