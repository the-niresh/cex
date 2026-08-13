import { useCallback, useEffect, useState } from "react";
import { Auth } from "./components/Auth";
import { Balances } from "./components/Balances";
import { ActivityPanel } from "./components/ActivityPanel";
import { Chart } from "./components/Chart";
import { InstrumentStrip } from "./components/InstrumentStrip";
import { OrderBook } from "./components/OrderBook";
import type { BookTab } from "./components/PanelTabs";
import { Tape } from "./components/Tape";
import { Ticket } from "./components/Ticket";
import { TopBar } from "./components/TopBar";
import { Panel } from "./components/ui/panel";
import { latencySeries as readLatencySeries, latencyStats, onLatency } from "./lib/api";
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
  const [latencySeries, setLatencySeries] = useState(readLatencySeries);
  useEffect(
    () =>
      onLatency((next) => {
        setLatency(next);
        setLatencySeries(readLatencySeries());
      }),
    [],
  );


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
      <div
        // `group/screen` so a panel can read the stale flag off the shell —
        // `group-data-[stale=true]/screen:` — instead of the flag being drilled
        // through as a prop to every panel that dims when the feed does.
        className={[
          "group/screen grid h-screen gap-[5px] bg-bg p-[5px]",
          // chart · book-or-trades · ticket. The book and the tape share a
          // column through the tabs in their panel head, which is what took the
          // floor from 1353px to 1112px — most of the width problem solved by
          // showing one of two things that were never read at the same moment.
          //
          // The first row is `auto`, not `36px`: the topbar folds rather than
          // clips, and a folded topbar needs the row to grow with it. The last
          // row is fixed rather than `auto` because `auto` lets the grid
          // squeeze the instrument strip under 100vh and clip its sparklines.
          "grid-cols-[minmax(420px,1fr)_370px_320px] grid-rows-[auto_minmax(0,1fr)_132px_60px]",
          // Three columns want 1112px. Below that the ticket column would fall
          // off the side of a screen that cannot scroll, taking BUY, SELL and
          // LOG OUT with it — and a 1280px viewport is exactly what a 1920
          // screen at 150% zoom gives you, which is not an exotic setup. So the
          // same three columns, tightened, down to an 852px floor.
          "max-[1111px]:grid-cols-[minmax(260px,1fr)_minmax(300px,370px)_minmax(290px,320px)]",
          // Under ~880px no side-by-side arrangement leaves the ladder or the
          // ticket a usable width, so stop pretending: one column, each panel
          // its own row, and the document scrolls like any other page.
          "max-[879px]:h-auto max-[879px]:min-h-screen",
          "max-[879px]:grid-cols-[minmax(0,1fr)] max-[879px]:grid-rows-none max-[879px]:auto-rows-auto",
        ].join(" ")}
        data-testid="screen"
        data-stale={looksStale ? "true" : "false"}
      >
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

        {/* The right rail carries the ticket, balances and deposit stacked.
            Three panels in one column need ~545px; under that — a 1366×768
            laptop once the browser has taken its chrome — the deposit block
            used to fall out of the bottom of a clipped container, so funding
            an account became impossible on a short screen. Scroll, not clip. */}
        <Panel className="overflow-y-auto" data-testid="ticket-rail">
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
        </Panel>

        <ActivityPanel
          orders={x.openOrders}
          fills={x.fills}
          markets={x.markets}
          onCancel={(id) => void x.cancel(id)}
        />

        <InstrumentStrip
          stats={latency}
          series={latencySeries}
          depthSeq={x.depthSeq}
          resyncs={x.resyncs}
          status={x.status}
          silentForMs={silentFor}
        />
      </div>

      {authOpen && x.session === null && <Auth onSubmit={signIn} onClose={closeAuth} />}

      {/* A failed request, dismissible, never blocking the book behind it. */}
      {x.error && (
        <div
          className={[
            "fixed bottom-[30px] right-3 z-[60] flex max-w-[420px] items-start gap-2.5 px-2.5 py-2",
            "rounded-panel bg-panel-hi text-[10.5px] leading-[1.45] text-ink",
            "shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-sell)_45%,transparent)]",
          ].join(" ")}
          role="alert"
        >
          <span className="flex-none pt-px font-sans text-[9.5px] font-bold tracking-[0.14em] text-sell">
            ERR
          </span>
          <span>{x.error}</span>
          <button
            type="button"
            className="ml-auto flex-none cursor-pointer text-ink-4 hover:text-ink"
            onClick={x.clearError}
            aria-label="dismiss"
          >
            ✕
          </button>
        </div>
      )}
    </>
  );
}
