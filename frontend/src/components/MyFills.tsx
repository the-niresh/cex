import { decimalsForStep, feeAsset, formatAtoms } from "../lib/num";
import type { Market, MyFill } from "../lib/types";
import { Num, clock } from "./format";
import { ColumnHeads, Empty, Meta, Panel, PanelHead, PanelTitle, Scroll, Table } from "./ui/panel";

/** Headings and rows share one column template, so they cannot drift apart. */
const COLS = [
  "grid-cols-[66px_92px_56px_minmax(72px,1fr)_80px_88px] gap-x-2",
  "[&>:nth-child(n+4)]:text-right",
].join(" ");

/** Under this the table scrolls sideways rather than crushing its columns. */
const MIN_WIDTH = 470;

export function MyFills({
  fills,
  markets,
  bare = false,
}: {
  fills: MyFill[];
  markets: Market[];
  /** Rendered inside the tabbed activity panel, which supplies the chrome. */
  bare?: boolean;
}) {
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

  const width = fills.length === 0 ? undefined : MIN_WIDTH;

  const table = (
    <Table>
      <ColumnHeads className={COLS} style={{ minWidth: width }} data-testid="fill-heads">
        <span>Time</span>
        <span>Market</span>
        <span>Side</span>
        <span>Price</span>
        <span>Qty</span>
        <span>Fee</span>
      </ColumnHeads>
      <Scroll style={{ minWidth: width }}>
        {fills.length === 0 ? (
          <Empty>no fills yet</Empty>
        ) : (
          fills.map((fill) => {
            const market = bySymbol.get(fill.symbol);
            const priceDp = market ? decimalsForStep(market.tick_size, market.quote_decimals) : 2;
            const qtyDp = market ? decimalsForStep(market.lot_size, market.base_decimals) : 5;
            const buy = fill.side === "BUY";
            return (
              <div
                key={`${fill.seq}-${fill.idx}`}
                className={`tnum grid h-[19px] items-center px-2.5 hover:bg-row-hover ${COLS}`}
                data-testid="fill-row"
              >
                <span className="text-[10.5px] text-ink-4">{clock(fill.timestamp_ms)}</span>
                <span className="font-sans tracking-[0.04em] text-ink-2">{fill.symbol}</span>
                <span
                  className={`whitespace-nowrap font-sans text-micro tracking-[0.06em] ${
                    buy ? "text-buy" : "text-sell"
                  }`}
                  data-testid="fill-side"
                >
                  {fill.side}{" "}
                  {/* maker/taker rides on the side cell rather than paying for
                      a column of its own */}
                  <span className="tracking-[0.04em] text-ink-4" data-testid="fill-role">
                    {fill.role === "MAKER" ? "M" : "T"}
                  </span>
                </span>
                <span className={buy ? "text-buy" : "text-sell"}>
                  {market && (
                    <Num atoms={fill.price} decimals={market.quote_decimals} places={priceDp} />
                  )}
                </span>
                <span className="text-ink-2">
                  {market && <Num atoms={fill.qty} decimals={market.base_decimals} places={qtyDp} />}
                </span>
                <span>
                  {market && <Num atoms={fill.fee} decimals={feeAsset(fill.side, market).decimals} />}
                </span>
              </div>
            );
          })
        )}
      </Scroll>
    </Table>
  );

  if (bare) return <div className="flex min-h-0 flex-1 flex-col">{table}</div>;

  return (
    <Panel data-testid="fills-panel">
      <PanelHead>
        <PanelTitle>My fills</PanelTitle>
        {feeText && (
          <Meta>
            fees <b className="tnum font-medium text-ink-2">{feeText}</b>
          </Meta>
        )}
      </PanelHead>
      {table}
    </Panel>
  );
}
