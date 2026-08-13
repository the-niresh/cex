import { decimalsForStep, feeAsset, formatAtoms } from "../lib/num";
import type { Market, MyFill } from "../lib/types";
import { Num, clock } from "./format";

export function MyFills({ fills, markets }: { fills: MyFill[]; markets: Market[] }) {
  const bySymbol = new Map(markets.map((m) => [m.symbol, m]));

  // A fee is charged in whatever that side received — base for a buy, quote
  // for a sell — so one market can produce fees in two assets. Group by the
  // asset actually paid and show only what is unambiguous; adding BTC to USDT
  // because both came from BTC_USDT would be a number that means nothing.
  const feeByAsset = new Map<string, { total: bigint; decimals: bigint }>();
  for (const fill of fills) {
    const market = bySymbol.get(fill.symbol);
    if (!market) continue;
    const { asset, decimals } = feeAsset(fill.side, market);
    const entry = feeByAsset.get(asset) ?? { total: 0n, decimals };
    entry.total += fill.fee;
    feeByAsset.set(asset, entry);
  }
  // At each asset's own scale rather than a fixed two places: a fee in BTC is
  // a handful of atoms, and rounding it to 2dp prints "0.00" — a summary that
  // says you paid nothing when you did.
  const feeText = [...feeByAsset.entries()]
    .map(([asset, { total, decimals }]) => `${formatAtoms(total, decimals)} ${asset}`)
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
                  data-testid="fill-row"
                >
                  <span className="t">{clock(fill.timestamp_ms)}</span>
                  <span className="mkt">{fill.symbol}</span>
                  <span className="side" data-testid="fill-side">
                    {fill.side}{" "}
                    <span className="role" data-testid="fill-role">
                      {fill.role === "MAKER" ? "M" : "T"}
                    </span>
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
                    {market && (
                      <Num atoms={fill.fee} decimals={feeAsset(fill.side, market).decimals} />
                    )}
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
