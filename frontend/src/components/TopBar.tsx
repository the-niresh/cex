import type { FeedStatus } from "../lib/feed";
import { decimalsForStep, formatAtoms } from "../lib/num";
import type { Market, Session } from "../lib/types";
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
  session: Session | null;
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
  session,
  onSignOut,
}: Props) {
  const degraded = stale || status !== "live";

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
        <div className="ticker">
          <div className="field">
            <span className="k">last</span>
            <span className={`last${lastSide === "SELL" ? " down" : ""}`}>
              {lastPrice === null ? (
                "—"
              ) : (
                <Num
                  atoms={lastPrice}
                  decimals={market.quote_decimals}
                  places={decimalsForStep(market.tick_size, market.quote_decimals)}
                />
              )}
            </span>
          </div>
          <div className="field">
            <span className="k">tick</span>
            <span className="v">
              {formatAtoms(market.tick_size, market.quote_decimals, {
                places: decimalsForStep(market.tick_size, market.quote_decimals),
              })}
            </span>
          </div>
          <div className="field">
            <span className="k">lot</span>
            <span className="v">
              {formatAtoms(market.lot_size, market.base_decimals, {
                places: decimalsForStep(market.lot_size, market.base_decimals),
              })}
            </span>
          </div>
          <div className="field">
            <span className="k">fees mkr/tkr</span>
            <span className="v">
              {String(market.maker_fee_bps)} / {String(market.taker_fee_bps)} <span className="d">bps</span>
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
        <span className="id">{session ? `${session.user_id.slice(0, 8)}…` : "signed out"}</span>
        {session && <button onClick={onSignOut}>LOG OUT</button>}
      </div>
    </header>
  );
}
