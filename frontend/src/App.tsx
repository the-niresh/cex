import { useCallback, useEffect, useState } from "react";
import { Auth } from "./components/Auth";
import { Balances } from "./components/Balances";
import { Chart } from "./components/Chart";
import { MyFills } from "./components/MyFills";
import { OpenOrders } from "./components/OpenOrders";
import { OrderBook } from "./components/OrderBook";
import type { BookTab } from "./components/PanelTabs";
import { Tape } from "./components/Tape";
import { Ticket } from "./components/Ticket";
import { TopBar } from "./components/TopBar";
import { API_URL, latencyStats, onLatency } from "./lib/api";
import { WS_URL } from "./lib/feed";
import { ENGINE_P99_BASELINE_MS } from "./lib/latency";
import { decimalsForStep, formatAtoms } from "./lib/num";
import type { AuthMode, Credentials } from "./lib/types";
import { useExchange } from "./useExchange";

/** After this long without an update the book is called stale, not live. */
const STALE_AFTER_MS = 8_000;

export default function App() {
  const x = useExchange();
  const [price, setPrice] = useState("");
  const [now, setNow] = useState(() => Date.now());

  // The book and the tape share a column; this decides which is showing.
  const [bookTab, setBookTab] = useState<BookTab>("book");

  const [latency, setLatency] = useState(latencyStats);
  useEffect(() => onLatency(setLatency), []);

  // A slow exchange is a degraded connection, so it gets the same amber the
  // socket already uses. The threshold is the measured p99, not a taste call.
  const engineDegraded =
    latency.engineP50 !== null && latency.engineP50 > ENGINE_P99_BASELINE_MS;

  // The screen is public. Nothing asks for an account until something is
  // about to move money, and then this opens — never on arrival.
  const [authOpen, setAuthOpen] = useState(false);
  const openAuth = useCallback(() => setAuthOpen(true), []);
  const closeAuth = useCallback(() => setAuthOpen(false), []);

  // Not memoised: the panel only calls this from its submit handler, never
  // from a dependency array, so a stable identity would buy nothing.
  async function signIn(mode: AuthMode, credentials: Credentials) {
    // Only reached on success — a failure throws and the panel shows why.
    await x.signIn(mode, credentials);
    setAuthOpen(false);
  }

  // A book nobody has updated for a while is not necessarily broken, but the
  // user must be able to tell. Ticking drives that readout.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

  const lastPrint = x.tape[0] ?? null;
  const bestBid = x.bids[0]?.price ?? null;
  const bestAsk = x.asks[0]?.price ?? null;

  const silentFor = x.lastUpdateMs === null ? null : now - x.lastUpdateMs;
  const looksStale =
    x.bookStale || x.status !== "live" || (silentFor !== null && silentFor > STALE_AFTER_MS);

  const staleReason = x.bookStale
    ? "STALE — SEQUENCE GAP, RESYNCING"
    : x.status !== "live"
      ? `STALE — SOCKET ${x.status.toUpperCase()}`
      : silentFor !== null
        ? `NO UPDATES FOR ${Math.floor(silentFor / 1000)}S`
        : null;

  return (
    <>
      <div className={`screen${looksStale ? " stale" : ""}`}>
        <TopBar
          markets={x.markets}
          market={x.market}
          symbol={x.symbol}
          onSelect={x.selectMarket}
          lastPrice={lastPrint?.price ?? null}
          lastSide={lastPrint?.taker_side ?? null}
          status={x.status}
          stale={x.bookStale}
          day={x.day}
          session={x.session}
          onSignIn={openAuth}
          onSignOut={x.signOut}
        />

        <Chart
          market={x.market}
          candles={x.candles}
          interval={x.interval}
          onInterval={x.setInterval}
        />

        {/* One column, two readings of the same market. */}
        {bookTab === "book" ? (
          <OrderBook
            market={x.market}
            bids={x.bids}
            asks={x.asks}
            spread={x.spread}
            depthSeq={x.depthSeq}
            stale={looksStale}
            staleReason={staleReason}
            mine={x.mine}
            lastPrice={lastPrint?.price ?? null}
            lastSide={lastPrint?.taker_side ?? null}
            tab={bookTab}
            onTab={setBookTab}
            onPickPrice={(picked) => {
              if (!x.market) return;
              setPrice(
                formatAtoms(picked, x.market.quote_decimals, {
                  places: decimalsForStep(x.market.tick_size, x.market.quote_decimals),
                  group: false,
                }),
              );
            }}
          />
        ) : (
          <Tape market={x.market} prints={x.tape} tab={bookTab} onTab={setBookTab} />
        )}

        <section className="panel ticket bal">
          <Ticket
            market={x.market}
            balances={x.balances}
            price={price}
            onPriceChange={setPrice}
            bestBid={bestBid}
            bestAsk={bestAsk}
            signedIn={x.session !== null}
            onRequireSignIn={openAuth}
            onSubmit={x.submitOrder}
          />
          <Balances
            balances={x.balances}
            markets={x.markets}
            signedIn={x.session !== null}
            onRequireSignIn={openAuth}
            onDeposit={x.credit}
          />
        </section>

        <OpenOrders orders={x.openOrders} markets={x.markets} onCancel={(id) => void x.cancel(id)} />

        <MyFills fills={x.fills} markets={x.markets} />

        <footer className="statusbar">
          <span>
            api <b>{new URL(API_URL).host}</b>
          </span>
          <span className="sep">│</span>
          <span>
            ws <b>{new URL(WS_URL).host}</b> <span className="ok">{x.status}</span>
          </span>
          <span className="sep">│</span>
          <span>
            depth_seq <b>{x.depthSeq === null ? "—" : String(x.depthSeq)}</b>
          </span>
          {/* Two numbers, never a sum. They are measured differently and
              adding them would be the misleading figure this exists to avoid. */}
          <span className="sep">│</span>
          <span>
            engine{" "}
            <b className={engineDegraded ? "warn" : undefined}>
              {latency.engineP50 === null ? "—" : `${latency.engineP50}ms`}
            </b>
          </span>
          <span className="sep">│</span>
          <span>
            network <b>{latency.networkP50 === null ? "—" : `${latency.networkP50}ms`}</b>
          </span>
          <div className="right">
            <span>
              resyncs <b>{x.resyncs}</b>
            </span>
            <span>
              updated <b>{silentFor === null ? "—" : `${Math.floor(silentFor / 1000)}s ago`}</b>
            </span>
          </div>
        </footer>
      </div>

      {authOpen && x.session === null && <Auth onSubmit={signIn} onClose={closeAuth} />}

      {x.error && (
        <div className="toast" role="alert">
          <span className="k">ERR</span>
          <span>{x.error}</span>
          <button onClick={x.clearError} aria-label="dismiss">
            ✕
          </button>
        </div>
      )}
    </>
  );
}
