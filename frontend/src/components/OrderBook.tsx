import type { Level } from "../lib/book";
import { decimalsForStep } from "../lib/num";
import type { Market } from "../lib/types";
import { Num } from "./format";

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
  onPickPrice(price: bigint): void;
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
  onPickPrice,
}: Props) {
  if (!market) {
    return (
      <section className="panel book">
        <div className="phead">
          <h2>Order book</h2>
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

  const row = (level: Level, max: bigint) => {
    const own = mine.get(level.price);
    const width = max > 0n ? Number((level.total * 10_000n) / max) / 100 : 0;
    return (
      <div
        key={String(level.price)}
        className={`lvl${own ? " has-mine" : ""}`}
        onClick={() => onPickPrice(level.price)}
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
        <span className="price">
          <Num atoms={level.price} decimals={market.quote_decimals} places={priceDp} />
        </span>
        <span className="num size">
          <Num atoms={level.qty} decimals={market.base_decimals} places={qtyDp} />
        </span>
        <span className="num cum">
          <Num atoms={level.total} decimals={market.base_decimals} places={qtyDp} />
        </span>
      </div>
    );
  };

  const spreadBps =
    spread !== null && lastPrice && lastPrice > 0n
      ? Number((spread * 1_000_000n) / lastPrice) / 100
      : null;

  return (
    <section className="panel book">
      <div className="phead">
        <h2>Order book</h2>
        <span className="meta">
          {market.symbol} · depth_seq <b>{depthSeq === null ? "—" : String(depthSeq)}</b>
        </span>
      </div>

      {stale && (
        <div className="staleband">
          <span>{staleReason ?? "STALE — RESYNCING"}</span>
        </div>
      )}

      <div className="chead">
        <span>Mine</span>
        <span>Price</span>
        <span>Size</span>
        <span>Total</span>
      </div>

      <div className="ladder">
        {/* Worst ask at the top, so the best one sits against the spread. */}
        <div className="side asks">{[...asks].reverse().map((l) => row(l, maxAsk))}</div>

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

        <div className="side bids">{bids.map((l) => row(l, maxBid))}</div>
      </div>
    </section>
  );
}
