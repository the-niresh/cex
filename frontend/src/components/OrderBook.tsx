import { useState } from "react";
import type { Level } from "../lib/book";
import { decimalsForStep, notional } from "../lib/num";
import type { Market } from "../lib/types";
import { Num } from "./format";
import { PanelTabs, type BookTab } from "./PanelTabs";

interface Props {
  market: Market | null;
  bids: Level[];
  asks: Level[];
  spread: bigint | null;
  depthSeq: bigint | null;
  stale: boolean;
  staleReason: string | null;
  mine: Map<bigint, bigint>;
  lastPrice: bigint | null;
  lastSide: "BUY" | "SELL" | null;
  tab: BookTab;
  onTab(next: BookTab): void;
  onPickPrice(price: bigint): void;
}

/**
 * What sweeping the book to a given level would actually cost you.
 *
 * The ladder already shows cumulative *size*; this is the other half — the
 * cash, and the average price it works out to. Both come from levels already
 * on screen, so hovering answers "what if I took all of that" without a
 * request, and without the user doing the arithmetic in their head.
 */
interface Sweep {
  /**
   * Which half is being pointed at. The card parks in the *other* half, so it
   * never sits on top of the row you are reading, and never jumps around under
   * the cursor the way a tooltip that follows the mouse does.
   */
  from: "asks" | "bids";
  /** Cumulative base atoms down to and including the hovered level. */
  size: bigint;
  /** Cumulative quote atoms for the same levels. */
  value: bigint;
  /** `value / size`, in quote atoms — the blended price of the sweep. */
  avg: bigint;
}

export function OrderBook({
  market,
  bids,
  asks,
  spread,
  depthSeq,
  stale,
  staleReason,
  mine,
  lastPrice,
  lastSide,
  tab,
  onTab,
  onPickPrice,
}: Props) {
  const [sweep, setSweep] = useState<Sweep | null>(null);

  if (!market) {
    return (
      <section className="panel book" data-testid="book-panel">
        <div className="phead">
          <PanelTabs tab={tab} onTab={onTab} />
        </div>
        <div className="empty">waiting for markets</div>
      </section>
    );
  }

  const priceDp = decimalsForStep(market.tick_size, market.quote_decimals);
  const qtyDp = decimalsForStep(market.lot_size, market.base_decimals);

  // Each side scales against its own deepest level, so a thin book on one side
  // still shows shape instead of a flat line.
  const maxAsk = asks.length ? (asks[asks.length - 1] as Level).total : 1n;
  const maxBid = bids.length ? (bids[bids.length - 1] as Level).total : 1n;

  /**
   * Running quote value down each side, best price first.
   *
   * The engine's levels carry cumulative *size* but not cumulative cash, and
   * the cash is what a taker actually cares about. Rounded up per level, the
   * same direction the engine charges, so the figure is never flattering.
   */
  const runningValue = (side: Level[]): bigint[] => {
    const out: bigint[] = [];
    let running = 0n;
    for (const level of side) {
      running += notional(level.price, level.qty, market.base_decimals, "up");
      out.push(running);
    }
    return out;
  };
  const askValue = runningValue(asks);
  const bidValue = runningValue(bids);

  const row = (level: Level, max: bigint, value: bigint, from: "asks" | "bids") => {
    const own = mine.get(level.price);
    const width = max > 0n ? Number((level.total * 10_000n) / max) / 100 : 0;
    return (
      <div
        key={String(level.price)}
        className={`lvl${own ? " has-mine" : ""}`}
        // Identity in the testid, state in the data attributes. A test asks for
        // `[data-testid="ladder-level"][data-mine="true"]` rather than a class,
        // so restyling cannot silently unhook the suite from the thing it
        // checks. `has-mine` stays because the stylesheet still hangs off it.
        data-testid="ladder-level"
        data-side={from}
        data-mine={own ? "true" : "false"}
        onClick={() => onPickPrice(level.price)}
        onMouseEnter={() =>
          setSweep({
            from,
            size: level.total,
            value,
            avg:
              level.total > 0n ? (value * 10n ** market.base_decimals) / level.total : level.price,
          })
        }
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") onPickPrice(level.price);
        }}
      >
        <i className="bar" style={{ width: `${width}%` }} />
        <span className="mine">
          {own ? <Num atoms={own} decimals={market.base_decimals} places={qtyDp} /> : ""}
        </span>
        <span className="price" data-testid="level-price">
          <Num atoms={level.price} decimals={market.quote_decimals} places={priceDp} />
        </span>
        <span className="num size" data-testid="level-size">
          <Num atoms={level.qty} decimals={market.base_decimals} places={qtyDp} />
        </span>
        <span className="num cum" data-testid="level-total">
          <Num atoms={level.total} decimals={market.base_decimals} places={qtyDp} />
        </span>
      </div>
    );
  };

  const spreadBps =
    spread !== null && lastPrice && lastPrice > 0n
      ? Number((spread * 1_000_000n) / lastPrice) / 100
      : null;

  // A market nobody has quoted yet is a normal state, not a broken panel. The
  // tape and the chart both say so when they are empty; the ladder used to
  // render its headings over a void, which is the one panel big enough for
  // that to read as a failed screen rather than a quiet market.
  const empty = bids.length === 0 && asks.length === 0;

  // Resting size on each side of the visible ladder, as a share of the two.
  // Visible, not whole-book: it is read off the same levels on screen, so the
  // bar and the rows above it can never disagree.
  const bidDepth = bids.length ? (bids[bids.length - 1] as Level).total : 0n;
  const askDepth = asks.length ? (asks[asks.length - 1] as Level).total : 0n;
  const depth = bidDepth + askDepth;
  const bidShare = depth > 0n ? Number((bidDepth * 10_000n) / depth) / 100 : 50;

  return (
    <section className="panel book" data-testid="book-panel">
      <div className="phead">
        <PanelTabs tab={tab} onTab={onTab} />
        <span className="meta">
          depth_seq <b>{depthSeq === null ? "—" : String(depthSeq)}</b>
        </span>
      </div>

      {stale && (
        <div className="staleband">
          <span>{staleReason ?? "STALE — RESYNCING"}</span>
        </div>
      )}

      <div className="chead" data-testid="ladder-heads">
        <span>Mine</span>
        <span>Price</span>
        <span>Size</span>
        <span>Total</span>
      </div>

      <div className="ladder" onMouseLeave={() => setSweep(null)}>
        {/* Worst ask at the top, so the best one sits against the spread. */}
        <div className="side asks">
          {empty ? (
            <div className="empty">nothing offered</div>
          ) : (
            asks.map((level, i) => row(level, maxAsk, askValue[i] as bigint, "asks")).reverse()
          )}
        </div>

        <div className="spread">
          <span className={`last${lastSide === "SELL" ? " down" : ""}`}>
            {lastPrice === null ? (
              "—"
            ) : (
              <Num atoms={lastPrice} decimals={market.quote_decimals} places={priceDp} />
            )}
          </span>
          <div className="gap">
            <span className="k">spread</span>
            <span className="v">
              {spread === null ? (
                "—"
              ) : (
                <>
                  <Num atoms={spread} decimals={market.quote_decimals} places={priceDp} />
                  {spreadBps !== null && ` · ${spreadBps.toFixed(2)} bps`}
                </>
              )}
            </span>
          </div>
        </div>

        <div className="side bids">
          {empty ? (
            <div className="empty">no bids yet</div>
          ) : (
            bids.map((level, i) => row(level, maxBid, bidValue[i] as bigint, "bids"))
          )}
        </div>

        {sweep && (
          <div className={`sweep ${sweep.from === "bids" ? "high" : "low"}`} data-testid="sweep">
            <div className="r">
              <span className="k">avg price</span>
              <span className="v" data-testid="sweep-avg-price">
                <Num atoms={sweep.avg} decimals={market.quote_decimals} places={priceDp} />
              </span>
            </div>
            <div className="r">
              <span className="k">sum {market.base}</span>
              <span className="v" data-testid="sweep-sum-base">
                <Num atoms={sweep.size} decimals={market.base_decimals} places={qtyDp} />
              </span>
            </div>
            <div className="r">
              <span className="k">sum {market.quote}</span>
              <span className="v" data-testid="sweep-sum-quote">
                <Num atoms={sweep.value} decimals={market.quote_decimals} places={2} />
              </span>
            </div>
          </div>
        )}
      </div>

      {/* Which side is holding more size, at a glance. The two labels come from
          one rounding, not two: rounding each half separately puts 59.5 and
          40.5 on screen as "60%" and "41%", which adds to 101. The bar itself
          keeps the exact widths. */}
      <div className="imbalance" aria-label="resting size, bids against asks" data-testid="imbalance">
        <div className="b" style={{ flexBasis: `${bidShare}%` }}>
          <span>{Math.round(bidShare)}%</span>
        </div>
        <div className="s" style={{ flexBasis: `${100 - bidShare}%` }}>
          <span>{100 - Math.round(bidShare)}%</span>
        </div>
      </div>
    </section>
  );
}
