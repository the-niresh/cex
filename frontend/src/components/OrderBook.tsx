import { useState } from "react";
import type { Level } from "../lib/book";
import { decimalsForStep, notional } from "../lib/num";
import type { Market } from "../lib/types";
import { Num } from "./format";
import { PanelTabs, type BookTab } from "./PanelTabs";
import { ColumnHeads, Empty, Meta, Panel, PanelHead } from "./ui/panel";

/**
 * Headings and rows share one column template, so they cannot drift apart.
 * Numbers cluster right in one tight block; the left is left to the depth
 * histogram, so bar and figure never fight for the same pixels.
 */
const COLS = "grid-cols-[1fr_92px_74px_82px] gap-x-2 [&>span]:text-right";

/**
 * How far the depth bar may reach, measured leftward from the row's right edge:
 * the row's right padding, then the Total and Size columns and the gaps between
 * them. A bar at 100% therefore stops in the gap before the Price column and
 * never runs under a price.
 *
 * ⚠️ It has to be a length, not a percentage. As `width: 100%` the deepest
 * level swept the whole row — which was tolerable while the bar grew from the
 * left away from the numbers, and stopped being tolerable the moment it was
 * anchored right, because then the bar's *strongest* end is the end sitting on
 * the price. Every column it spans is fixed, so this stays correct at any panel
 * width; only the Mine column takes the slack.
 */
const DEPTH_TRACK_PX = 10 + 82 + 8 + 74 + 8;

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
      <Panel className="max-stack:min-h-[430px]" data-testid="book-panel">
        <PanelHead>
          <PanelTabs tab={tab} onTab={onTab} />
        </PanelHead>
        <Empty>waiting for markets</Empty>
      </Panel>
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
    const asks = from === "asks";
    return (
      <div
        key={String(level.price)}
        className={[
          "group/lvl tnum relative grid h-[22px] flex-none cursor-pointer items-center border-l-2 px-2.5 text-micro",
          COLS,
          "hover:bg-hover",
          // The bar sits behind; everything else is lifted over it.
          "[&>span]:relative [&>span]:z-[1]",
          // A level you have resting in it is marked on the leading edge, in
          // its own side's hue — the one place in the ladder that is about you
          // rather than about the market.
          own ? (asks ? "border-l-sell" : "border-l-buy") : "border-l-transparent",
        ].join(" ")}
        // Identity in the testid, state in the data attributes. A test asks for
        // `[data-testid="ladder-level"][data-mine="true"]` rather than a class,
        // so restyling cannot silently unhook the suite from the thing it checks.
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
        {/* Depth bar: anchored to the right edge, growing leftward under the
            size and total figures and stopping short of the price column, so
            the price column stays clear at every depth. Cumulative, not level
            size — that is what actually informs. It needs enough weight for the
            eye to read shape down the ladder before it reads any number. */}
        <i
          className={`absolute inset-y-px right-0 z-0 border-l ${
            asks ? "border-l-sell/55 bg-sell/22" : "border-l-buy/55 bg-buy/22"
          }`}
          style={{ width: `${(width * DEPTH_TRACK_PX) / 100}px` }}
        />
        <span className={own ? "text-ink-2" : "text-ink-4"}>
          {own ? <Num atoms={own} decimals={market.base_decimals} places={qtyDp} /> : ""}
        </span>
        <span
          className={`underline-offset-[3px] group-hover/lvl:underline ${
            asks ? "text-sell" : "text-buy"
          }`}
          data-testid="level-price"
        >
          <Num atoms={level.price} decimals={market.quote_decimals} places={priceDp} />
        </span>
        <span className="text-ink" data-testid="level-size">
          <Num atoms={level.qty} decimals={market.base_decimals} places={qtyDp} />
        </span>
        <span className="text-ink-3" data-testid="level-total">
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
    <Panel className="max-stack:min-h-[430px]" data-testid="book-panel">
      <PanelHead>
        <PanelTabs tab={tab} onTab={onTab} />
        <Meta>
          depth_seq{" "}
          <b className="tnum font-medium text-ink-2">
            {depthSeq === null ? "—" : String(depthSeq)}
          </b>
        </Meta>
      </PanelHead>

      {/* Two conditions, two treatments — the same split `feedHealth` already
          makes, which the band used to flatten.

          `degraded` means the book cannot be trusted: hatched amber, and the
          ladder below goes flat and grey. Anything else here is a *quiet*
          market — the prices are correct, nobody has traded — and that gets a
          plain grey note and no dimming at all. Dimming correct data is how you
          teach someone to distrust a screen that is telling the truth, and on a
          venue with little traffic the quiet case is the normal one. */}
      {stale && (
        <div
          className={[
            "flex h-5 flex-none items-center gap-2 px-2.5",
            "font-sans text-micro",
            "text-ink-4 border-b border-rule",
            "group-data-[degraded=true]/screen:text-warn",
            "group-data-[degraded=true]/screen:border-warn/35",
            "group-data-[degraded=true]/screen:bg-[repeating-linear-gradient(-45deg,color-mix(in_oklab,var(--color-warn)_11%,transparent)_0_6px,transparent_6px_12px)]",
          ].join(" ")}
          data-testid="staleband"
        >
          <span>{staleReason ?? "Stale — resyncing"}</span>
        </div>
      )}

      <ColumnHeads className={COLS} data-testid="ladder-heads">
        <span>Mine</span>
        <span>Price ({market.quote})</span>
        <span>Size ({market.base})</span>
        <span>Total ({market.base})</span>
      </ColumnHeads>

      {/* `relative` so the sweep card can be placed against the half being
          pointed at. It goes flat and grey when the book cannot be trusted —
          `degraded`, not merely quiet. Keyed off `stale`, this dimmed a correct
          book every time a market went eight seconds without a trade, which on
          the deployed venue is most of the time. */}
      <div
        className="relative flex min-h-0 flex-1 flex-col group-data-[degraded=true]/screen:saturate-[.15] group-data-[degraded=true]/screen:brightness-[.62]"
        data-testid="ladder-body"
        onMouseLeave={() => setSweep(null)}
      >
        {/* Worst ask at the top, so the best one sits against the spread. */}
        <div className="flex min-h-0 flex-1 flex-col justify-end overflow-hidden">
          {empty ? (
            <Empty>nothing offered</Empty>
          ) : (
            asks.map((level, i) => row(level, maxAsk, askValue[i] as bigint, "asks")).reverse()
          )}
        </div>

        <div className="flex h-[38px] flex-none items-center gap-3.5 border-y border-rule-hi bg-panel-hi px-2.5">
          {/* 15px, not the top bar's 22px. Both readouts are the same number in
              the same green, and two headline prices on one screen means
              neither is the headline. The top bar's is captioned and sits with
              the day's stats, so it keeps the size; this one only has to
              out-rank the 12px ladder rows it separates. */}
          <span
            className={`tnum text-[15px] font-medium ${
              lastSide === "SELL" ? "text-sell" : "text-buy"
            }`}
          >
            {lastPrice === null ? (
              "—"
            ) : (
              <>
                <Num atoms={lastPrice} decimals={market.quote_decimals} places={priceDp} />
                <span className="align-[2px] text-micro">
                  {lastSide === "SELL" ? " ▼" : " ▲"}
                </span>
              </>
            )}
          </span>
          <div className="ml-auto text-right leading-[1.2]">
            <span className="block font-sans text-micro text-ink-4">Spread</span>
            <span className="tnum text-micro text-ink-2">
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

        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          {empty ? (
            <Empty>no bids yet</Empty>
          ) : (
            bids.map((level, i) => row(level, maxBid, bidValue[i] as bigint, "bids"))
          )}
        </div>

        {/* What taking everything down to the hovered level would cost. It
            parks in the half you are *not* pointing at — hovering a bid puts it
            up among the asks and vice versa — so it never covers the rows being
            read and never chases the cursor. */}
        {sweep && (
          <div
            className={[
              "pointer-events-none absolute right-2.5 z-[3] min-w-[172px] px-2 py-[5px]",
              "bg-panel-hi shadow-[inset_0_0_0_1px_var(--color-rule-hi),0_6px_18px_rgb(0_0_0/.45)]",
              sweep.from === "bids" ? "top-1.5" : "bottom-1.5",
            ].join(" ")}
            data-testid="sweep"
          >
            <SweepRow label="Avg price" testId="sweep-avg-price">
              <Num atoms={sweep.avg} decimals={market.quote_decimals} places={priceDp} />
            </SweepRow>
            <SweepRow label={`Sum ${market.base}`} testId="sweep-sum-base">
              <Num atoms={sweep.size} decimals={market.base_decimals} places={qtyDp} />
            </SweepRow>
            <SweepRow label={`Sum ${market.quote}`} testId="sweep-sum-quote">
              <Num atoms={sweep.value} decimals={market.quote_decimals} places={2} />
            </SweepRow>
          </div>
        )}
      </div>

      {/* Which side is holding more size, at a glance. The two labels come from
          one rounding, not two: rounding each half separately puts 59.5 and
          40.5 on screen as "60%" and "41%", which adds to 101. The bar itself
          keeps the exact widths. */}
      {/* Resting size, bids against asks. Reads as one bar, not two panels. */}
      <div
        className="tnum flex h-4 flex-none border-t border-rule text-micro [&>div]:flex [&>div]:min-w-0 [&>div]:items-center [&>div]:overflow-hidden"
        aria-label="resting size, bids against asks"
        data-testid="imbalance"
      >
        <div className="justify-start bg-buy/22 pl-2.5 text-buy" style={{ flexBasis: `${bidShare}%` }}>
          <span>{Math.round(bidShare)}%</span>
        </div>
        <div
          className="justify-end bg-sell/22 pr-2.5 text-sell"
          style={{ flexBasis: `${100 - bidShare}%` }}
        >
          <span>{100 - Math.round(bidShare)}%</span>
        </div>
      </div>
    </Panel>
  );
}

/** One line of the sweep card: a caption and the figure it names. */
function SweepRow({
  label,
  testId,
  children,
}: {
  label: string;
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-baseline gap-3 leading-[1.5]">
      <span className="font-sans text-micro text-ink-4">{label}</span>
      <span className="tnum ml-auto text-micro text-ink" data-testid={testId}>
        {children}
      </span>
    </div>
  );
}
