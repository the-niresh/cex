import { decimalsForStep, formatAtoms } from "../lib/num";
import type { Market, Order } from "../lib/types";
import { Num } from "./format";

interface Props {
  orders: Order[];
  markets: Market[];
  onCancel(orderId: bigint): void;
}

export function OpenOrders({ orders, markets, onCancel }: Props) {
  const bySymbol = new Map(markets.map((m) => [m.symbol, m]));

  // What the resting orders are holding, so the number in the header explains
  // the gap between `available` and what you thought you had.
  const locked = new Map<string, bigint>();
  for (const order of orders) {
    const market = bySymbol.get(order.symbol);
    if (!market || order.price === null) continue;
    const remaining = order.qty - order.filled_qty;
    if (order.side === "BUY") {
      const cost = (order.price * remaining) / 10n ** market.base_decimals;
      locked.set(market.quote, (locked.get(market.quote) ?? 0n) + cost);
    } else {
      locked.set(market.base, (locked.get(market.base) ?? 0n) + remaining);
    }
  }
  const lockedText = [...locked.entries()]
    .map(([asset, amount]) => {
      const market = markets.find((m) => m.quote === asset || m.base === asset);
      const decimals = market
        ? market.quote === asset
          ? market.quote_decimals
          : market.base_decimals
        : 8n;
      return `${formatAtoms(amount, decimals, { places: 2 })} ${asset}`;
    })
    .join(" + ");

  return (
    <section className="panel open-orders">
      <div className="phead">
        <h2>Open orders</h2>
        <span className="meta">
          {orders.length} live{lockedText && <> · <b>{lockedText}</b> locked</>}
        </span>
      </div>
      <div className="chead">
        <span>ID</span>
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
      </div>
      <div className="scroll">
        {orders.length === 0 ? (
          <div className="empty">nothing resting</div>
        ) : (
          orders.map((order) => {
            const market = bySymbol.get(order.symbol);
            const priceDp = market ? decimalsForStep(market.tick_size, market.quote_decimals) : 2;
            const qtyDp = market ? decimalsForStep(market.lot_size, market.base_decimals) : 5;
            const progress =
              order.qty > 0n ? Number((order.filled_qty * 10_000n) / order.qty) / 100 : 0;
            const partial = order.filled_qty > 0n;

            return (
              <div key={String(order.order_id)} className={`oo ${order.side.toLowerCase()}`}>
                <span className="id">#{String(order.order_id)}</span>
                <span className="age">—</span>
                <span className="mkt">{order.symbol}</span>
                <span className="side">{order.side}</span>
                <span className="kind">{order.order_type}</span>
                <span className="num">
                  {order.price === null || !market ? (
                    "MKT"
                  ) : (
                    <Num atoms={order.price} decimals={market.quote_decimals} places={priceDp} />
                  )}
                </span>
                <span className="num">
                  {market && <Num atoms={order.qty} decimals={market.base_decimals} places={qtyDp} />}
                </span>
                <span className="num filled">
                  {market && (
                    <Num atoms={order.filled_qty} decimals={market.base_decimals} places={qtyDp} />
                  )}
                </span>
                <span className="prog">
                  <i style={{ width: `${progress}%` }} />
                </span>
                <span className={`st${partial ? " partial" : ""}`}>{order.status}</span>
                <button
                  className="x"
                  onClick={() => onCancel(order.order_id)}
                  aria-label={`cancel order ${order.order_id}`}
                >
                  ✕
                </button>
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}
