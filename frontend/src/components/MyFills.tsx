import { sentenceCase } from "../lib/labels";
import { decimalsForStep, feeAsset } from "../lib/num";
import type { Market, MyFill } from "../lib/types";
import { Num, clockMs } from "./format";
import { ColumnHeads, Empty, Scroll, Table } from "./ui/panel";

/**
 * Headings and rows share one column template, so they cannot drift apart.
 *
 * Every data column is fixed and a filler column takes whatever is left. The
 * slack used to sit on Price as `minmax(72px,1fr)`, which was invisible while
 * the panel was half the screen — once the activity panel went full width it
 * opened a ~700px hole between Side and Price. Nothing in a fills row is worth
 * stretching, so the table packs left and the empty space lands past the last
 * column where it reads as margin rather than as a missing cell.
 */
const COLS = [
  "grid-cols-[88px_92px_82px_92px_80px_88px_minmax(0,1fr)] gap-x-2",
  "[&>:nth-child(n+4)]:text-right",
].join(" ");

/** Under this the table scrolls sideways rather than crushing its columns. */
const MIN_WIDTH = 590;

export function MyFills({ fills, markets }: { fills: MyFill[]; markets: Market[] }) {
  const bySymbol = new Map(markets.map((m) => [m.symbol, m]));

  const width = fills.length === 0 ? undefined : MIN_WIDTH;

  return (
    <Table>
      <ColumnHeads className={COLS} style={{ minWidth: width }} data-testid="fill-heads">
        <span>Time</span>
        <span>Market</span>
        <span>Side</span>
        <span>Price</span>
        <span>Qty</span>
        <span>Fee</span>
        <span />
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
            // Millis, like the tape. One market order can print twice inside a
            // second, and to the second those two rows were indistinguishable.
            const { time, millis } = clockMs(fill.timestamp_ms);
            return (
              <div
                key={`${fill.seq}-${fill.idx}`}
                className={`tnum grid h-6 items-center px-2.5 hover:bg-row-hover ${COLS}`}
                data-testid="fill-row"
              >
                <span className="text-micro text-ink-4">
                  {time}
                  <span className="text-ink-3">{millis}</span>
                </span>
                <span className="font-sans text-ink-2">{fill.symbol}</span>
                <span
                  className={`whitespace-nowrap font-sans text-micro ${
                    buy ? "text-buy" : "text-sell"
                  }`}
                  data-testid="fill-side"
                >
                  {fill.side}{" "}
                  {/* maker/taker rides on the side cell rather than paying for
                      a column of its own. Spelt out: "BUY T" was a two-letter
                      code nothing on the screen defined. */}
                  <span className="text-ink-4" data-testid="fill-role">
                    {sentenceCase(fill.role)}
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
                <span />
              </div>
            );
          })
        )}
      </Scroll>
    </Table>
  );
}
