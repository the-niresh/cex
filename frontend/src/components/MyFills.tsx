import { decimalsForStep, formatAtoms } from "../lib/num";
import type { Market, MyFill } from "../lib/types";
import { Num, clock } from "./format";

export function MyFills({ fills, markets }: { fills: MyFill[]; markets: Market[] }) {
  const bySymbol = new Map(markets.map((m) => [m.symbol, m]));

  // Fees are per-market in that market's quote asset. Every market here quotes
  // in USDT, but summing blindly across quote assets would still be wrong, so
  // group first and show only what is unambiguous.
  const feeByQuote = new Map<string, { total: bigint; decimals: bigint }>();
  for (const fill of fills) {
    const market = bySymbol.get(fill.symbol);
    if (!market) continue;
    const entry = feeByQuote.get(market.quote) ?? { total: 0n, decimals: market.quote_decimals };
    entry.total += fill.fee;
    feeByQuote.set(market.quote, entry);
  }
  const feeText = [...feeByQuote.entries()]
    .map(([asset, { total, decimals }]) => `${formatAtoms(total, decimals, { places: 2 })} ${asset}`)
    .join(" + ");

  return (
    <section className="panel fills">
      <div className="phead">
        <h2>My fills</h2>
        {feeText && (
          <span className="meta">
            fees <b>{feeText}</b>
          </span>
        )}
      </div>
      {/* One scroller for headings and rows together — see OpenOrders. */}
      <div className="tbl">
        <div className="chead">
          <span>Time</span>
          <span>Market</span>
          <span>Side</span>
          <span>Price</span>
          <span>Qty</span>
          <span>Fee</span>
        </div>
        <div className="scroll">
          {fills.length === 0 ? (
            <div className="empty">no fills yet</div>
          ) : (
            fills.map((fill) => {
              const market = bySymbol.get(fill.symbol);
              const priceDp = market ? decimalsForStep(market.tick_size, market.quote_decimals) : 2;
              const qtyDp = market ? decimalsForStep(market.lot_size, market.base_decimals) : 5;
              return (
                <div
                  key={`${fill.seq}-${fill.idx}`}
                  className={`fl ${fill.side.toLowerCase()}`}
                >
                  <span className="t">{clock(fill.timestamp_ms)}</span>
                  <span className="mkt">{fill.symbol}</span>
                  <span className="side">
                    {fill.side} <span className="role">{fill.role === "MAKER" ? "M" : "T"}</span>
                  </span>
                  <span className="num p">
                    {market && (
                      <Num atoms={fill.price} decimals={market.quote_decimals} places={priceDp} />
                    )}
                  </span>
                  <span className="num q">
                    {market && <Num atoms={fill.qty} decimals={market.base_decimals} places={qtyDp} />}
                  </span>
                  <span className="num f">
                    {market && <Num atoms={fill.fee} decimals={market.quote_decimals} />}
                  </span>
                </div>
              );
            })
          )}
        </div>
      </div>
    </section>
  );
}
