import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import * as api from "../lib/api";
import { summarizePortfolio } from "../lib/portfolio";
import { decimalsForAsset } from "../lib/deposit";
import { decimalsForStep } from "../lib/num";
import type { Market } from "../lib/types";
import { useExchangeContext } from "../ExchangeContext";
import { ActivityPanel } from "./ActivityPanel";
import { DepositDialog } from "./DepositDialog";
import { Num } from "./format";
import { ColumnHeads, Empty, Meta, PanelHead, PanelTitle, Scroll } from "./ui/panel";
import { GhostButton } from "./ui/form";

const COLS =
  "grid-cols-[44px_1fr_88px_88px_100px_100px_64px] gap-x-2 [&>span:not(:first-child)]:text-right";

async function loadLastPrices(markets: Market[]): Promise<Map<string, bigint>> {
  const prices = new Map<string, bigint>([["USDT", 1_000_000n]]);
  await Promise.all(
    markets.map(async (m) => {
      try {
        const trades = await api.trades(m.symbol, 1);
        if (trades[0]) prices.set(m.base, trades[0].price);
      } catch {
        // Leave without a price; the summary will exclude it.
      }
    }),
  );
  return prices;
}

export function PortfolioPage() {
  const x = useExchangeContext();
  const [prices, setPrices] = useState<Map<string, bigint>>(new Map([["USDT", 1_000_000n]]));
  const [depositOpen, setDepositOpen] = useState(false);

  const lastTapeKey = x.tape[0]?.key ?? null;

  useEffect(() => {
    if (x.markets.length === 0) return;
    let cancelled = false;
    void loadLastPrices(x.markets).then((next) => {
      if (!cancelled) setPrices(next);
    });
    return () => {
      cancelled = true;
    };
  }, [x.markets, x.symbol, lastTapeKey]);

  const summary = useMemo(
    () => summarizePortfolio(x.balances, prices, x.markets),
    [x.balances, prices, x.markets],
  );

  if (!x.session) {
    return (
      <div className="flex min-h-screen flex-col items-center justify-center gap-3 bg-bg p-6 text-center">
        <p className="font-sans text-label text-ink-2">Try as guest to view your portfolio.</p>
        <div className="flex flex-wrap justify-center gap-2">
          <button
            type="button"
            className="rounded-control bg-field px-3 py-1.5 font-sans text-micro text-ink hover:bg-hover"
            data-testid="portfolio-try-guest"
            onClick={() => void x.signInAsGuest()}
          >
            Try as guest
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-screen flex-col gap-2 bg-bg p-2" data-testid="portfolio-page">
      <header className="flex items-center justify-between rounded-panel border border-rule bg-panel-hi px-3 py-2">
        <div className="flex items-center gap-3">
          <Link to="/" className="font-sans text-micro text-ink-4 hover:text-ink-2">Trade</Link>
          <span className="font-sans text-label font-medium text-ink">Portfolio</span>
        </div>
        <GhostButton onClick={() => setDepositOpen(true)} data-testid="portfolio-deposit">
          Deposit
        </GhostButton>
      </header>

      <section className="rounded-panel border border-rule bg-panel p-4" data-testid="portfolio-total">
        <div className="font-sans text-micro text-ink-4">Total value</div>
        <div className="tnum text-[28px] font-medium text-ink">
          $<Num atoms={summary.total} decimals={6n} places={2} />
        </div>
        {summary.withoutPrice.length > 0 && (
          <p className="mt-2 font-sans text-micro text-ink-4" data-testid="portfolio-no-price-note">
            {summary.withoutPrice.join(", ")} excluded from total (no price)
          </p>
        )}
      </section>

      <div className="flex min-h-0 flex-1 flex-col rounded-panel border border-rule bg-panel">
        <PanelHead>
          <PanelTitle>Holdings</PanelTitle>
          <Meta>{summary.rows.length} assets</Meta>
        </PanelHead>
        <ColumnHeads className={COLS}>
          <span>Asset</span>
          <span />
          <span>Available</span>
          <span>Locked</span>
          <span>Last</span>
          <span>Value</span>
          <span>Share</span>
        </ColumnHeads>
        <Scroll className="min-h-14">
          {summary.rows.length === 0 ? (
            <Empty>no balances yet</Empty>
          ) : (
            summary.rows.map((row) => {
              const dp = decimalsForAsset(row.asset, x.markets);
              const market = x.markets.find((m) => m.base === row.asset);
              const priceDp = market
                ? decimalsForStep(market.tick_size, market.quote_decimals)
                : row.asset === "USDT"
                  ? 2
                  : 2;
              return (
                <div
                  key={row.asset}
                  className={`tnum grid h-5 items-center px-2.5 hover:bg-row-hover ${COLS}`}
                  data-testid="portfolio-row"
                  data-asset={row.asset}
                >
                  <span className="font-sans text-ink-2">{row.asset}</span>
                  <span />
                  <span><Num atoms={row.available} decimals={dp} /></span>
                  <span className={row.locked === 0n ? "text-ink-4" : "text-ink-3"}>
                    <Num atoms={row.locked} decimals={dp} />
                  </span>
                  <span>
                    {row.lastPrice === null ? (
                      <span className="text-ink-4">no price</span>
                    ) : row.asset === "USDT" ? (
                      "1.00"
                    ) : (
                      <Num atoms={row.lastPrice} decimals={6n} places={priceDp} />
                    )}
                  </span>
                  <span>
                    {row.value === null ? (
                      <span className="text-ink-4">-</span>
                    ) : (
                      <Num atoms={row.value} decimals={6n} places={2} />
                    )}
                  </span>
                  <span className="text-ink-3">
                    {row.sharePct === null ? "-" : `${row.sharePct.toFixed(1)}%`}
                  </span>
                </div>
              );
            })
          )}
        </Scroll>
      </div>

      <ActivityPanel
        orders={x.openOrders}
        fills={x.fills}
        markets={x.markets}
        onCancel={(id) => void x.cancel(id)}
      />

      {depositOpen && (
        <DepositDialog
          markets={x.markets}
          signedIn={x.session !== null}
          onGuest={() => void x.signInAsGuest()}
          onDeposit={x.credit}
          onClose={() => setDepositOpen(false)}
        />
      )}


    </div>
  );
}
