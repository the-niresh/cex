/**
 * Atomic-unit arithmetic and formatting.
 *
 * Nothing here converts to `number`. Every value the exchange sends is an
 * integer count of the smallest indivisible unit of an asset, and the whole
 * point of carrying them as `bigint` is that no step of the way rounds.
 * Formatting happens by slicing digits, not by dividing.
 *
 *   - a **quantity** counts `10^-base_decimals` of the base asset, so with
 *     BTC's 8 decimals `100000` is `0.001 BTC`.
 *   - a **price** counts quote atoms per *one whole* base unit, so with USDT's
 *     6 decimals `50000000000` is `50,000.00 USDT`.
 *   - therefore `notional = price × qty / 10^base_decimals`, in quote atoms.
 */

import type { Market, Side } from "./types";

export function pow10(n: bigint): bigint {
  return 10n ** n;
}

/** Rounding direction, named for who absorbs the remainder. */
export type Rounding = "down" | "up";

function divide(numerator: bigint, denominator: bigint, rounding: Rounding): bigint {
  const quotient = numerator / denominator;
  if (rounding === "down" || numerator % denominator === 0n) return quotient;
  return quotient + (numerator < 0n ? -1n : 1n);
}

/**
 * Quote atoms for `qty` base atoms at `price`.
 *
 * `rounding` decides who absorbs the sub-atom remainder. The engine charges a
 * buyer the rounded-up cost and credits a seller the rounded-down proceeds, so
 * the exchange is never left short; the ticket shows the same number the engine
 * will charge rather than a prettier one.
 */
export function notional(
  price: bigint,
  qty: bigint,
  baseDecimals: bigint,
  rounding: Rounding = "up",
): bigint {
  return divide(price * qty, pow10(baseDecimals), rounding);
}

/** A fee on a notional, in basis points. Always rounded up, as the engine does. */
export function feeOn(notionalAtoms: bigint, bps: bigint): bigint {
  return divide(notionalAtoms * bps, 10_000n, "up");
}

/**
 * How many decimal places a tick or lot actually implies.
 *
 * A tick of `10000` quote atoms against USDT's 6 decimals is `0.01`, so prices
 * want two places — not six. Showing the full six would be honest but unreadable,
 * and showing an arbitrary two would be wrong for a market with a finer tick.
 */
/**
 * The asset a fill's fee was charged in, and the scale to read it at.
 *
 * A fee comes out of what the filler *received*, so it follows their own side:
 * `crates/core/src/state.rs` credits the buyer `qty - fee` in the base asset
 * and the seller `cost - fee` in the quote, paying the fee account in the same
 * asset each time. Base and quote rarely share a scale — BTC has 8 decimals,
 * USDT 6 — so reading every fee as quote silently renders a buyer's at a
 * hundred times its value, under the wrong ticker.
 */
export function feeAsset(
  side: Side,
  market: Pick<Market, "base" | "quote" | "base_decimals" | "quote_decimals">,
): { asset: string; decimals: bigint } {
  return side === "BUY"
    ? { asset: market.base, decimals: market.base_decimals }
    : { asset: market.quote, decimals: market.quote_decimals };
}

export function decimalsForStep(step: bigint, decimals: bigint): number {
  if (step <= 0n) return Number(decimals);
  let trailingZeros = 0n;
  let remaining = step;
  while (remaining % 10n === 0n && trailingZeros < decimals) {
    remaining /= 10n;
    trailingZeros += 1n;
  }
  return Number(decimals - trailingZeros);
}

const GROUP = /\B(?=(\d{3})+(?!\d))/g;

export interface FormatOptions {
  /** Decimal places to show. Defaults to the asset's full precision. */
  places?: number;
  /** Thousands separators on the integer part. On by default. */
  group?: boolean;
}

/**
 * Render an atomic integer as a decimal string, by slicing digits.
 *
 * Never divides, so nothing rounds on the way to the screen. `places` shorter
 * than the asset's precision truncates rather than rounds — a displayed balance
 * must never read higher than the balance actually is.
 */
export function formatAtoms(
  atoms: bigint,
  decimals: bigint,
  { places, group = true }: FormatOptions = {},
): string {
  const negative = atoms < 0n;
  const digits = (negative ? -atoms : atoms).toString().padStart(Number(decimals) + 1, "0");

  const cut = digits.length - Number(decimals);
  let whole = digits.slice(0, cut);
  let fraction = digits.slice(cut);

  const want = places ?? Number(decimals);
  fraction = want <= 0 ? "" : fraction.slice(0, want).padEnd(want, "0");

  if (group) whole = whole.replace(GROUP, ",");

  const sign = negative ? "-" : "";
  return fraction ? `${sign}${whole}.${fraction}` : `${sign}${whole}`;
}

/**
 * Read a typed amount into atomic units.
 *
 * Returns `null` for anything that is not a plain decimal number, and for more
 * decimal places than the asset has — silently dropping a digit the user typed
 * is how an order ends up for a different size than they meant.
 */
export function parseAtoms(input: string, decimals: bigint): bigint | null {
  const cleaned = input.trim().replace(/,/g, "");
  if (!/^\d*(\.\d*)?$/.test(cleaned) || cleaned === "" || cleaned === ".") return null;

  const [whole, fraction = ""] = cleaned.split(".");
  if (fraction.length > Number(decimals)) return null;

  return BigInt(whole || "0") * pow10(decimals) + BigInt(fraction.padEnd(Number(decimals), "0") || "0");
}

/** Whether a value sits on a tick or lot boundary. */
export function isAligned(value: bigint, step: bigint): boolean {
  return step > 0n && value % step === 0n;
}

/** The nearest allowed value at or below `value`. */
export function floorToStep(value: bigint, step: bigint): bigint {
  if (step <= 0n) return value;
  return value - (value % step);
}

/** Basis points as a readable percentage, e.g. `5n` → `"0.05%"`. */
export function bpsToPercent(bps: bigint): string {
  return `${formatAtoms(bps, 2n, { group: false })}%`;
}
