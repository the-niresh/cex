import type { FeedStatus } from "../lib/feed";
import { decimalsForStep } from "../lib/num";
import type { DayStats, Market, Session } from "../lib/types";
import { Num } from "./format";

interface Props {
  markets: Market[];
  market: Market | null;
  symbol: string;
  onSelect(symbol: string): void;
  lastPrice: bigint | null;
  lastSide: "BUY" | "SELL" | null;
  status: FeedStatus;
  stale: boolean;
  day: DayStats | null;
  session: Session | null;
  onSignIn(): void;
  onSignOut(): void;
}

const STATUS_TEXT: Record<FeedStatus, string> = {
  connecting: "CONNECTING",
  live: "LIVE",
  reconnecting: "RECONNECTING",
  closed: "OFFLINE",
};

export function TopBar({
  markets,
  market,
  symbol,
  onSelect,
  lastPrice,
  lastSide,
  status,
  stale,
  day,
  session,
  onSignIn,
  onSignOut,
}: Props) {
  const degraded = stale || status !== "live";
  const priceDp = market ? decimalsForStep(market.tick_size, market.quote_decimals) : 2;
  const qtyDp = market ? decimalsForStep(market.lot_size, market.base_decimals) : 5;

  return (
    <header className="panel topbar">
      <div className="brand">
        <span className="mark">CEX</span>
        <span className="build">spot</span>
      </div>

      <nav className="markets">
        {markets.map((m) => (
          <button key={m.symbol} aria-selected={m.symbol === symbol} onClick={() => onSelect(m.symbol)}>
            <span className="sym">{m.symbol}</span>
            <span className="lp">
              {m.symbol === symbol && lastPrice !== null ? (
                <span className={lastSide === "SELL" ? "dn" : "up"}>
                  <Num
                    atoms={lastPrice}
                    decimals={m.quote_decimals}
                    places={decimalsForStep(m.tick_size, m.quote_decimals)}
                  />
                </span>
              ) : (
                "—"
              )}
            </span>
          </button>
        ))}
      </nav>

      {market && (
        /* The day, not the rule book. Tick, lot and fees used to live here and
           are all still stated where they bite — beside the price and quantity
           inputs, and in the ticket's fee readout — whereas nothing on the
           screen said what the market had actually done since yesterday. */
        <div className="ticker">
          <div className="field">
            <span className="k">last</span>
            <span className={`last${lastSide === "SELL" ? " down" : ""}`}>
              {lastPrice === null ? (
                "—"
              ) : (
                <Num atoms={lastPrice} decimals={market.quote_decimals} places={priceDp} />
              )}
            </span>
          </div>
          <div className="field">
            <span className="k">24h change</span>
            <span className={`v chg${day === null ? "" : day.change < 0n ? " dn" : " up"}`}>
              {day === null ? (
                "—"
              ) : (
                <>
                  {day.change < 0n ? "−" : "+"}
                  <Num
                    atoms={day.change < 0n ? -day.change : day.change}
                    decimals={market.quote_decimals}
                    places={priceDp}
                  />
                  {day.changePct !== null && (
                    <span className="pct">
                      {day.changePct < 0 ? "" : "+"}
                      {day.changePct.toFixed(2)}%
                    </span>
                  )}
                </>
              )}
            </span>
          </div>
          <div className="field">
            <span className="k">24h high</span>
            <span className="v">
              {day === null ? (
                "—"
              ) : (
                <Num atoms={day.high} decimals={market.quote_decimals} places={priceDp} />
              )}
            </span>
          </div>
          <div className="field">
            <span className="k">24h low</span>
            <span className="v">
              {day === null ? (
                "—"
              ) : (
                <Num atoms={day.low} decimals={market.quote_decimals} places={priceDp} />
              )}
            </span>
          </div>
          <div className="field">
            <span className="k">24h volume</span>
            <span className="v">
              {day === null ? (
                "—"
              ) : (
                <>
                  <Num atoms={day.volume} decimals={market.base_decimals} places={qtyDp} />{" "}
                  <span className="d">{market.base}</span>
                </>
              )}
            </span>
          </div>
        </div>
      )}

      <div className="conn">
        <i className="dot" style={degraded ? { background: "var(--warn)", animation: "none" } : undefined} />
        <span className="txt" style={degraded ? { color: "var(--warn)" } : undefined}>
          {stale ? "RESYNCING" : STATUS_TEXT[status]}
        </span>
      </div>

      <div className="who">
        {session?.name && <span className="name">{session.name}</span>}
        <span className="id">{session ? `${session.user_id.slice(0, 8)}…` : "signed out"}</span>
        <button onClick={session ? onSignOut : onSignIn}>{session ? "LOG OUT" : "LOG IN"}</button>
      </div>
    </header>
  );
}
