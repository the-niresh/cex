import { useEffect, useState } from "react";
import { Auth } from "./components/Auth";
import { Balances } from "./components/Balances";
import { Chart } from "./components/Chart";
import { MyFills } from "./components/MyFills";
import { OpenOrders } from "./components/OpenOrders";
import { OrderBook } from "./components/OrderBook";
import { Tape } from "./components/Tape";
import { Ticket } from "./components/Ticket";
import { TopBar } from "./components/TopBar";
import { API_URL } from "./lib/api";
import { WS_URL } from "./lib/feed";
import { decimalsForStep, formatAtoms } from "./lib/num";
import { useExchange } from "./useExchange";

/** After this long without an update the book is called stale, not live. */
const STALE_AFTER_MS = 8_000;

export default function App() {
  const x = useExchange();
  const [price, setPrice] = useState("");
  const [now, setNow] = useState(() => Date.now());

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
          session={x.session}
          onSignOut={x.signOut}
        />

        <Chart
          market={x.market}
          candles={x.candles}
          interval={x.interval}
          onInterval={x.setInterval}
        />

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

        <Tape market={x.market} prints={x.tape} />

        <section className="panel ticket bal">
          <Ticket
            market={x.market}
            balances={x.balances}
            price={price}
            onPriceChange={setPrice}
            bestBid={bestBid}
            bestAsk={bestAsk}
            disabled={x.session === null}
            onSubmit={x.submitOrder}
          />
          <Balances balances={x.balances} markets={x.markets} onDeposit={x.credit} />
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

      {x.session === null && <Auth onSubmit={x.signIn} />}

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
