import { feeAsset, formatAtoms } from "./num";
import type { Market, MyFill, Order } from "./types";

/**
 * The two one-line summaries the activity panel puts beside its tabs.
 *
 * Both were computed inside the tables that display the rows, in a branch that
 * rendered a standalone panel head — and once those tables moved into the
 * tabbed panel, nothing mounted that branch any more. The arithmetic kept
 * running and the answer went nowhere. Here it is testable and it has a reader.
 */

/**
 * What the resting orders are holding, per asset.
 *
 * Answers the question the balances panel provokes: why `available` is less
 * than what you thought you had. A bid holds quote — price times the unfilled
 * remainder — and an ask holds the coin itself.
 */
export function valueLocked(orders: Order[], markets: Market[]): string {
  const bySymbol = new Map(markets.map((m) => [m.symbol, m]));
  const locked = new Map<string, { total: bigint; decimals: bigint }>();

  for (const order of orders) {
    const market = bySymbol.get(order.symbol);
    // A market order rests at no price, so there is nothing to hold against it.
    if (!market || order.price === null) continue;

    const remaining = order.qty - order.filled_qty;
    const [asset, amount, decimals] =
      order.side === "BUY"
        ? [
            market.quote,
            (order.price * remaining) / 10n ** market.base_decimals,
            market.quote_decimals,
          ]
        : [market.base, remaining, market.base_decimals];

    const entry = locked.get(asset) ?? { total: 0n, decimals };
    entry.total += amount;
    locked.set(asset, entry);
  }

  return [...locked.entries()]
    .map(([asset, { total, decimals }]) => `${formatAtoms(total, decimals, { places: 2 })} ${asset}`)
    .join(" + ");
}

/**
 * What the fills cost, per asset.
 *
 * A fee is charged in whatever that side received — base for a buy, quote for a
 * sell — so one market can produce fees in two assets. Grouped by the asset
 * actually paid, and shown at that asset's own scale rather than a fixed two
 * places: a fee in BTC is a handful of atoms, and rounding it to 2dp prints
 * "0.00", a summary that says you paid nothing when you did.
 */
export function feesPaid(fills: MyFill[], markets: Market[]): string {
  const bySymbol = new Map(markets.map((m) => [m.symbol, m]));
  const byAsset = new Map<string, { total: bigint; decimals: bigint }>();

  for (const fill of fills) {
    const market = bySymbol.get(fill.symbol);
    if (!market) continue;
    const { asset, decimals } = feeAsset(fill.side, market);
    const entry = byAsset.get(asset) ?? { total: 0n, decimals };
    entry.total += fill.fee;
    byAsset.set(asset, entry);
  }

  return [...byAsset.entries()]
    .map(([asset, { total, decimals }]) => `${formatAtoms(total, decimals)} ${asset}`)
    .join(" + ");
}
