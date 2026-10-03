import { useEffect, useMemo, useState } from "react";
import * as api from "../lib/api";
import { assetColor, assetInitial } from "../lib/assets";
import { summarizePortfolio, assetValue } from "../lib/portfolio";
import { decimalsForAsset } from "../lib/deposit";
import { feedHealth } from "../lib/health";
import { decimalsForStep } from "../lib/num";
import type { Market } from "../lib/types";
import { useExchangeContext } from "../ExchangeContext";
import { ActivityPanel } from "./ActivityPanel";
import { DepositDialog } from "./DepositDialog";
import { Num } from "./format";
import { TopBar } from "./TopBar";
import { ActionButton } from "./ui/form";
import { ColumnHeads, Meta, Panel, PanelHead, PanelTitle, Scroll } from "./ui/panel";

const COLS =
  "grid-cols-[minmax(120px,148px)_88px_88px_100px_100px_minmax(72px,96px)] gap-x-2 [&>span:not(:first-child)]:text-right";

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

function StatCard({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5 rounded-control border border-rule bg-field px-3 py-2">
      <span className="font-sans text-micro text-ink-4">{label}</span>
      <span className="tnum text-label font-medium text-ink-2">{children}</span>
    </div>
  );
}

function AssetBadge({ asset }: { asset: string }) {
  return (
    <span
      className="flex size-6 flex-none items-center justify-center rounded-full font-sans text-micro font-medium text-bg"
      style={{ backgroundColor: assetColor(asset) }}
      aria-hidden="true"
    >
      {assetInitial(asset)}
    </span>
  );
}

function AllocationBar({
  rows,
}: {
  rows: { asset: string; sharePct: number | null }[];
}) {
  const segments = rows.filter((r) => r.sharePct !== null && r.sharePct > 0);
  if (segments.length === 0) return null;

  return (
    <Panel data-testid="portfolio-allocation">
      <PanelHead>
        <PanelTitle>Allocation</PanelTitle>
      </PanelHead>
      <div className="flex flex-col gap-3 p-3">
        <div
          className="flex h-3 w-full overflow-hidden rounded-pill bg-field"
          role="img"
          aria-label="Portfolio allocation by asset"
        >
          {segments.map((row) => (
            <div
              key={row.asset}
              className="h-full min-w-px"
              style={{
                width: `${row.sharePct}%`,
                backgroundColor: assetColor(row.asset),
              }}
              title={`${row.asset} ${row.sharePct?.toFixed(1)}%`}
            />
          ))}
        </div>
        <div className="flex flex-wrap gap-x-3 gap-y-1">
          {segments.map((row) => (
            <span key={row.asset} className="flex items-center gap-1.5 font-sans text-micro text-ink-3">
              <i
                className="size-2 rounded-full"
                style={{ backgroundColor: assetColor(row.asset) }}
                aria-hidden="true"
              />
              {row.asset}
              <span className="tnum text-ink-4">{row.sharePct?.toFixed(1)}%</span>
            </span>
          ))}
        </div>
      </div>
    </Panel>
  );
}

export function PortfolioPage() {
  const x = useExchangeContext();
  const [prices, setPrices] = useState<Map<string, bigint>>(new Map([["USDT", 1_000_000n]]));
  const [depositOpen, setDepositOpen] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  const lastTapeKey = x.tape[0]?.key ?? null;
  const lastPrint = x.tape[0] ?? null;

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

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

  const silentFor = x.lastUpdateMs === null ? null : now - x.lastUpdateMs;
  const health = feedHealth({
    bookStale: x.bookStale,
    status: x.status,
    silentForMs: silentFor,
  });

  const usdtAvailable = x.balances.find((b) => b.asset === "USDT")?.available ?? 0n;
  const usdtDp = decimalsForAsset("USDT", x.markets);

  const lockedValue = useMemo(() => {
    let total = 0n;
    for (const row of summary.rows) {
      if (row.locked === 0n) continue;
      const price = prices.get(row.asset) ?? (row.asset === "USDT" ? 1_000_000n : null);
      const value = assetValue(row.locked, row.asset, price ?? null, x.markets);
      if (value !== null) total += value;
    }
    return total;
  }, [summary.rows, prices, x.markets]);

  const openDeposit = () => setDepositOpen(true);

  return (
    <>
      <div
        className={[
          "flex min-h-screen flex-col gap-2 bg-bg p-2",
          "max-stack:min-h-screen max-stack:h-auto",
        ].join(" ")}
        data-testid="portfolio-page"
      >
        <TopBar
          page="portfolio"
          markets={x.markets}
          market={x.market}
          symbol={x.symbol}
          onSelect={x.selectMarket}
          lastPrice={lastPrint?.price ?? null}
          lastSide={lastPrint?.taker_side ?? null}
          status={x.status}
          feedDegraded={health.degraded}
          day={x.day}
          session={x.session}
          onSignOut={x.signOut}
          onGuest={() => void x.signInAsGuest()}
          onDeposit={openDeposit}
        />

        <div className="mx-auto flex w-full max-w-[1200px] min-h-0 flex-1 flex-col gap-2">
          <section
            className="flex flex-wrap items-center gap-4 rounded-panel border border-rule bg-panel p-4"
            data-testid="portfolio-total"
          >
            <div className="min-w-[140px] flex-none">
              <div className="font-sans text-micro text-ink-4">Total value</div>
              <div className="tnum text-[28px] font-medium leading-tight text-ink">
                $<Num atoms={summary.total} decimals={6n} places={2} />
              </div>
            </div>

            <div className="flex flex-wrap gap-2">
              <StatCard label="Assets">
                <span className="tnum">{summary.rows.length}</span>
              </StatCard>
              <StatCard label="In open orders">
                $<Num atoms={lockedValue} decimals={6n} places={2} />
              </StatCard>
              <StatCard label="USDT available">
                <Num atoms={usdtAvailable} decimals={usdtDp} places={2} />
              </StatCard>
            </div>

            <ActionButton
              type="button"
              className="ml-auto h-9 min-w-[100px] shrink-0 px-4"
              data-testid="portfolio-deposit"
              onClick={openDeposit}
            >
              Deposit
            </ActionButton>
          </section>

          {summary.withoutPrice.length > 0 && (
            <p className="font-sans text-micro text-ink-4" data-testid="portfolio-no-price-note">
              {summary.withoutPrice.join(", ")} excluded from total (no price)
            </p>
          )}

          <div className="grid min-h-0 flex-1 grid-cols-1 gap-2 lg:grid-cols-[minmax(0,1fr)_260px]">
            <Panel className="min-h-[200px]">
              <PanelHead>
                <PanelTitle>Holdings</PanelTitle>
                <Meta>{summary.rows.length} assets</Meta>
              </PanelHead>
              <ColumnHeads className={COLS}>
                <span>Asset</span>
                <span>Available</span>
                <span>Locked</span>
                <span>Last</span>
                <span>Value</span>
                <span>Share</span>
              </ColumnHeads>
              <Scroll className="min-h-14">
                {summary.rows.length === 0 ? (
                  <div className="flex flex-col items-center gap-3 p-6 text-center">
                    <p className="font-sans text-micro text-ink-4">
                      {x.session ? "No balances yet. Deposit to get started." : "Sign in as guest to view holdings."}
                    </p>
                    <ActionButton
                      type="button"
                      className="h-9 px-4"
                      data-testid="portfolio-try-guest"
                      onClick={() => (x.session ? openDeposit() : void x.signInAsGuest())}
                    >
                      {x.session ? "Deposit" : "Try as guest"}
                    </ActionButton>
                  </div>
                ) : (
                  summary.rows.map((row) => {
                    const dp = decimalsForAsset(row.asset, x.markets);
                    const market = x.markets.find((m) => m.base === row.asset);
                    const priceDp = market
                      ? decimalsForStep(market.tick_size, market.quote_decimals)
                      : row.asset === "USDT"
                        ? 2
                        : 2;
                    const share = row.sharePct ?? 0;
                    return (
                      <div
                        key={row.asset}
                        className={`tnum grid h-7 items-center border-b border-rule px-2.5 hover:bg-row-hover ${COLS}`}
                        data-testid="portfolio-row"
                        data-asset={row.asset}
                      >
                        <span className="flex items-center gap-2 text-left font-sans text-ink-2">
                          <AssetBadge asset={row.asset} />
                          {row.asset}
                        </span>
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
                        <span className="flex items-center justify-end gap-1.5">
                          <span className="relative h-[3px] w-10 bg-rule">
                            <i
                              className="absolute inset-y-0 left-0 rounded-full"
                              style={{
                                width: `${Math.min(share, 100)}%`,
                                backgroundColor: assetColor(row.asset),
                              }}
                            />
                          </span>
                          <span className="min-w-[36px] text-ink-3">
                            {row.sharePct === null ? "-" : `${row.sharePct.toFixed(1)}%`}
                          </span>
                        </span>
                      </div>
                    );
                  })
                )}
              </Scroll>
            </Panel>

            <AllocationBar rows={summary.rows} />
          </div>

          <ActivityPanel
            className="min-h-[190px] shrink-0"
            orders={x.openOrders}
            fills={x.fills}
            markets={x.markets}
            onCancel={(id) => void x.cancel(id)}
          />
        </div>
      </div>

      {depositOpen && (
        <DepositDialog
          markets={x.markets}
          signedIn={x.session !== null}
          onGuest={() => void x.signInAsGuest()}
          onDeposit={x.credit}
          onClose={() => setDepositOpen(false)}
        />
      )}
    </>
  );
}
