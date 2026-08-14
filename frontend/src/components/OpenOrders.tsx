import { sentenceCase } from "../lib/labels";
import { decimalsForStep } from "../lib/num";
import type { Market, Order } from "../lib/types";
import { Num } from "./format";
import { ColumnHeads, Empty, Scroll, Table } from "./ui/panel";

interface Props {
  orders: Order[];
  markets: Market[];
  onCancel(orderId: bigint): void;
}

/**
 * id age market side type | price qty filled | progress status ✕
 *
 * Eleven columns, which is what the rows actually render — the template used
 * to declare fifteen (tif, remaining, notional, slack), so every heading from
 * Price rightwards sat over the wrong column and the table claimed 130px more
 * width than the panel had. Progress takes the slack; the rest are fixed so
 * the numbers stay in tabular alignment.
 */
const COLS = [
  "grid-cols-[56px_56px_92px_46px_52px_100px_92px_92px_minmax(40px,1fr)_80px_24px] gap-x-2",
  "[&>:nth-child(n+6)]:text-right [&>:nth-child(11)]:text-center",
].join(" ");

/**
 * Under this the table scrolls sideways rather than crushing its columns.
 *
 * Status went 76px → 80px when the raw enums became words: "Partially filled"
 * is the widest the engine can send and measures 77px at this font and size,
 * so 76 was one pixel short and anything above 80 is slack. ⚠️ Slack here is
 * not free — it is what decides whether this table fits the stacked layout at
 * 860px or scrolls sideways inside it.
 */
const MIN_WIDTH = 828;

export function OpenOrders({ orders, markets, onCancel }: Props) {
  const bySymbol = new Map(markets.map((m) => [m.symbol, m]));

  const width = orders.length === 0 ? undefined : MIN_WIDTH;

  return (
    <Table>
      <ColumnHeads className={COLS} style={{ minWidth: width }} data-testid="order-heads">
        <span>Id</span>
        <span>Age</span>
        <span>Market</span>
        <span>Side</span>
        <span>Type</span>
        <span>Price</span>
        <span>Qty</span>
        <span>Filled</span>
        <span>Progress</span>
        <span>Status</span>
        <span />
      </ColumnHeads>
      <Scroll style={{ minWidth: width }}>
        {orders.length === 0 ? (
          <Empty>nothing resting</Empty>
        ) : (
          orders.map((order) => {
            const market = bySymbol.get(order.symbol);
            const priceDp = market ? decimalsForStep(market.tick_size, market.quote_decimals) : 2;
            const qtyDp = market ? decimalsForStep(market.lot_size, market.base_decimals) : 5;
            const progress =
              order.qty > 0n ? Number((order.filled_qty * 10_000n) / order.qty) / 100 : 0;
            const buy = order.side === "BUY";

            return (
              <div
                key={String(order.order_id)}
                className={[
                  "tnum grid h-6 items-center border-l-2 px-2.5 hover:bg-row-hover",
                  COLS,
                  // The side is on the row's leading edge as well as in its
                  // own cell, so a column of orders reads as buys and sells
                  // before any of it is read as words.
                  buy ? "border-l-buy/60" : "border-l-sell/60",
                ].join(" ")}
                data-testid="open-order"
              >
                <span className="text-ink-4">#{String(order.order_id)}</span>
                <span className="text-ink-4">—</span>
                <span className="font-sans text-ink-2">{order.symbol}</span>
                <span
                  className={`font-sans text-micro ${buy ? "text-buy" : "text-sell"}`}
                  data-testid="order-side"
                >
                  {order.side}
                </span>
                <span className="font-sans text-micro text-ink-3">
                  {sentenceCase(order.order_type)}
                </span>
                <span>
                  {order.price === null || !market ? (
                    "MKT"
                  ) : (
                    <Num atoms={order.price} decimals={market.quote_decimals} places={priceDp} />
                  )}
                </span>
                <span>
                  {market && <Num atoms={order.qty} decimals={market.base_decimals} places={qtyDp} />}
                </span>
                <span className="text-ink-2">
                  {market && (
                    <Num atoms={order.filled_qty} decimals={market.base_decimals} places={qtyDp} />
                  )}
                </span>
                <span className="relative ml-3 h-[3px] bg-rule">
                  <i
                    className={`absolute inset-y-0 left-0 ${buy ? "bg-buy/80" : "bg-sell/80"}`}
                    style={{ width: `${progress}%` }}
                  />
                </span>
                <span
                  className={`font-sans text-micro ${
                    order.filled_qty > 0n ? "text-ink" : "text-ink-3"
                  }`}
                  data-testid="order-status"
                >
                  {sentenceCase(order.status)}
                </span>
                {/* 11px of glyph in a 24px row; the row is as tall as it gets,
                    so the target is widened instead to keep it clickable. */}
                <button
                  type="button"
                  className="flex min-w-6 cursor-pointer items-center justify-center self-stretch text-center text-[11px] leading-none text-ink-4 hover:text-sell"
                  data-testid="cancel-order"
                  onClick={() => onCancel(order.order_id)}
                  aria-label={`cancel order ${order.order_id}`}
                >
                  ✕
                </button>
              </div>
            );
          })
        )}
      </Scroll>
    </Table>
  );
}
