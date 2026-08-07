/**
 * Turning a live private fill into the same shape `/orders/history` returns.
 *
 * The two describe one event but arrive by different routes and at different
 * times. `ws` publishes the moment the engine matches; the history endpoint
 * reads a table `persist` writes asynchronously, so for a moment after your
 * own trade the REST view legitimately does not have it yet. Holding the live
 * one until the row appears is what stops your own fill being invisible until
 * you reload.
 */

import { notional } from "./num";
import type { Market, MyFill, OrderUpdate, Role, Side } from "./types";

/** `(seq, idx)` — the identity both the feed and the `fills` table use. */
export function fillKey(fill: Pick<MyFill, "seq" | "idx">): string {
  return `${fill.seq}-${fill.idx}`;
}

/**
 * Build the history row a live `fill` event will become, or `null` if it
 * cannot be built faithfully.
 *
 * Returns `null` rather than guessing when the market is unknown: `notional`
 * needs the base scale, and a fabricated one would be a wrong number rendered
 * as confidently as a right one.
 */
export function liveFillFrom(
  update: OrderUpdate,
  seq: bigint,
  markets: Market[],
): MyFill | null {
  if (update.event !== "fill") return null;
  const { order_id, symbol, side, price, qty, fee, role, idx } = update;
  if (
    order_id === undefined ||
    symbol === undefined ||
    side === undefined ||
    price === undefined ||
    qty === undefined ||
    fee === undefined ||
    role === undefined ||
    idx === undefined
  ) {
    return null;
  }

  const market = markets.find((m) => m.symbol === symbol);
  if (!market) return null;

  return {
    seq,
    idx,
    symbol,
    order_id,
    // `ws` serialises these lower-case and `/orders/history` upper. Both name
    // the same thing, and the screen compares them literally — an unnormalised
    // maker renders as a taker — so the boundary between wire and app is where
    // the two spellings have to become one.
    side: side.toUpperCase() as Side,
    role: role.toUpperCase() as Role,
    price,
    qty,
    notional: notional(price, qty, market.base_decimals),
    fee,
    // The engine owns no clock, so nothing upstream carries a trade time. The
    // tape already stamps live prints on arrival; this is the same choice, and
    // the history row that replaces it carries the real one.
    timestamp_ms: BigInt(Date.now()),
  };
}

/**
 * History plus any live fill it has not caught up to yet, newest first.
 *
 * Deduplicates on `(seq, idx)`, so a fill that has since been written lands
 * once — as the history row, which is the authority on it.
 */
export function mergeFills(history: MyFill[], live: MyFill[], limit: number): MyFill[] {
  const known = new Set(history.map(fillKey));
  return [...live.filter((f) => !known.has(fillKey(f))), ...history].slice(0, limit);
}
