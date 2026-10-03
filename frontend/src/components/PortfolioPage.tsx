import { useEffect, useMemo, useState } from "react";
import * as api from "../lib/api";
import { assetColor, assetFullName } from "../lib/assets";
import { decimalsForAsset } from "../lib/deposit";
import { feedHealth } from "../lib/health";
import { summarizePortfolio } from "../lib/portfolio";
import type { Candle, Market } from "../lib/types";
import { useExchangeContext } from "../ExchangeContext";
import { AssetIcon } from "./AssetIcon";
import { DepositDialog } from "./DepositDialog";
import { MyFills } from "./MyFills";
import { Num } from "./format";
import { OpenOrders } from "./OpenOrders";
import { PortfolioValueChart } from "./PortfolioValueChart";
import { TopBar } from "./TopBar";
import { ActionButton } from "./ui/form";
import { ColumnHeads, Scroll } from "./ui/panel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./ui/tabs";

const COLS = [
  "grid-cols-[148px_repeat(5,minmax(0,1fr))_72px]",
  "max-stack:grid-cols-[minmax(100px,128px)_repeat(3,minmax(0,1fr))]",
  "gap-x-4 max-stack:gap-x-3 [&>span:not(:first-child)]:text-right [&>div:not(:first-child)]:text-right",
].join(" ");

function compactPlaces(asset: string, markets: Market[]): number {
  if (asset === "USDT") return 2;
  return Math.min(6, Number(decimalsForAsset(asset, markets)));
}

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

async function loadHourlyCandles(markets: Market[]): Promise<Map<string, Candle[]>> {
  const out = new Map<string, Candle[]>();
  await Promise.all(
    markets.map(async (m) => {
      try {
        const candles = await api.candles(m.symbol, "1h", 24);
        out.set(m.symbol, candles);
      } catch {
        // Chart degrades gracefully without this market.
      }
    }),
  );
  return out;
}

function DollarValue({ atoms, muted }: { atoms: bigint | null; muted?: boolean }) {
  if (atoms === null) {
    return <span className={muted ? "text-ink-4" : "text-ink-3"}>-</span>;
  }
  return (
    <span className={muted ? "text-ink-4" : "text-ink-3"}>
      $<Num atoms={atoms} decimals={6n} places={2} />
    </span>
  );
}

function AmountCell({
  atoms,
  decimals,
  valueAtoms,
  quiet,
  compact,
}: {
  atoms: bigint;
  decimals: bigint;
  valueAtoms: bigint | null;
  quiet?: boolean;
  compact?: number;
}) {
  const muted = quiet || atoms === 0n;
  return (
    <div className="flex flex-col items-end leading-tight">
      <span className={muted ? "text-ink-4" : undefined}>
        <Num atoms={atoms} decimals={decimals} places={compact} />
      </span>
      <span className="font-sans text-micro max-stack:hidden">
        <DollarValue atoms={valueAtoms ?? (atoms === 0n ? 0n : null)} muted={muted} />
      </span>
    </div>
  );
}

function OverviewStat({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="font-sans text-micro text-ink-4">{label}</span>
      <span className="tnum text-label font-medium text-ink-2">{children}</span>
    </div>
  );
}

export function PortfolioPage() {
  const x = useExchangeContext();
  const [prices, setPrices] = useState<Map<string, bigint>>(new Map([["USDT", 1_000_000n]]));
  const [hourlyCandles, setHourlyCandles] = useState<Map<string, Candle[]>>(new Map());
  const [depositOpen, setDepositOpen] = useState(false);
  const [depositAsset, setDepositAsset] = useState("USDT");
  const [hideZero, setHideZero] = useState(false);
  const [tab, setTab] = useState("balances");
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

  useEffect(() => {
    if (x.markets.length === 0) return;
    let cancelled = false;
    void loadHourlyCandles(x.markets).then((next) => {
      if (!cancelled) setHourlyCandles(next);
    });
    return () => {
      cancelled = true;
    };
  }, [x.markets, lastTapeKey]);

  const summary = useMemo(
    () => summarizePortfolio(x.balances, prices, x.markets),
    [x.balances, prices, x.markets],
  );

  const visibleRows = useMemo(
    () => (hideZero ? summary.rows.filter((r) => r.total > 0n) : summary.rows),
    [summary.rows, hideZero],
  );

  const silentFor = x.lastUpdateMs === null ? null : now - x.lastUpdateMs;
  const health = feedHealth({
    bookStale: x.bookStale,
    status: x.status,
    silentForMs: silentFor,
  });

  const displayName = x.session?.name ?? x.session?.user_id ?? null;
  const avatarInitial = displayName?.charAt(0).toUpperCase() ?? "?";

  const openDeposit = (asset = "USDT") => {
    setDepositAsset(asset);
    setDepositOpen(true);
  };

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
          marketPrices={x.marketPrices}
          lastPrice={lastPrint?.price ?? null}
          lastSide={lastPrint?.taker_side ?? null}
          status={x.status}
          feedDegraded={health.degraded}
          day={x.day}
          session={x.session}
          onSignOut={x.signOut}
          onGuest={() => void x.signInAsGuest()}
          onDeposit={() => openDeposit()}
        />

        <div className="mx-auto flex w-full max-w-[1200px] flex-col gap-2">
          <section
            className="grid gap-4 rounded-panel border border-rule bg-panel p-4 lg:grid-cols-[minmax(0,1fr)_minmax(200px,280px)]"
            data-testid="portfolio-overview"
          >
            <div className="flex min-w-0 flex-col gap-3">
              {x.session ? (
                <div className="flex items-center gap-2.5">
                  <span
                    className="flex size-9 flex-none items-center justify-center rounded-full bg-field font-sans text-label font-medium text-ink"
                    aria-hidden="true"
                  >
                    {avatarInitial}
                  </span>
                  <span className="truncate font-sans text-data text-ink" data-testid="portfolio-account-name">
                    {displayName}
                  </span>
                </div>
              ) : (
                <div className="flex items-center gap-2.5">
                  <span
                    className="flex size-9 flex-none items-center justify-center rounded-full bg-field font-sans text-label text-ink-4"
                    aria-hidden="true"
                  >
                    ?
                  </span>
                  <span className="font-sans text-micro text-ink-4">Sign in to view your account</span>
                </div>
              )}

              <div>
                <div className="font-sans text-micro text-ink-4">Account balance</div>
                <div
                  className="tnum text-[28px] font-medium leading-tight text-ink"
                  data-testid="portfolio-total"
                >
                  $<Num atoms={summary.total} decimals={6n} places={2} />
                </div>
              </div>

              <div className="flex flex-wrap gap-x-6 gap-y-2">
                <OverviewStat label="Available">
                  $<Num atoms={summary.availableTotal} decimals={6n} places={2} />
                </OverviewStat>
                <OverviewStat label="In open orders">
                  $<Num atoms={summary.lockedTotal} decimals={6n} places={2} />
                </OverviewStat>
                <OverviewStat label="Assets held">
                  <span className="tnum">{summary.assetsHeld}</span>
                </OverviewStat>
              </div>
            </div>

            <PortfolioValueChart
              balances={x.balances}
              candlesBySymbol={hourlyCandles}
              markets={x.markets}
            />
          </section>

          {summary.withoutPrice.length > 0 && (
            <p className="font-sans text-micro text-ink-4" data-testid="portfolio-no-price-note">
              {summary.withoutPrice.join(", ")} excluded from total (no price)
            </p>
          )}

          <Tabs
            value={tab}
            onValueChange={(next) => setTab(String(next))}
            className="flex flex-col gap-0 overflow-hidden rounded-panel border border-rule bg-panel"
            data-testid="portfolio-tabs"
          >
            <div className="flex h-10 flex-none items-center gap-1 border-b border-rule bg-panel px-2">
              <TabsList className="h-auto gap-1 border-0 bg-transparent p-0">
                <TabsTrigger
                  value="balances"
                  data-testid="portfolio-tab-balances"
                  className="min-h-6 rounded-control px-2.5 font-sans text-label text-ink-4 transition-colors hover:text-ink-2 data-active:bg-field data-active:text-ink"
                >
                  Balances
                </TabsTrigger>
                <TabsTrigger
                  value="orders"
                  data-testid="portfolio-tab-orders"
                  className="min-h-6 rounded-control px-2.5 font-sans text-label text-ink-4 transition-colors hover:text-ink-2 data-active:bg-field data-active:text-ink"
                >
                  Open orders
                  <span className="tnum ml-1.5 text-micro text-ink-4">{x.openOrders.length}</span>
                </TabsTrigger>
                <TabsTrigger
                  value="fills"
                  data-testid="portfolio-tab-fills"
                  className="min-h-6 rounded-control px-2.5 font-sans text-label text-ink-4 transition-colors hover:text-ink-2 data-active:bg-field data-active:text-ink"
                >
                  Fills
                  <span className="tnum ml-1.5 text-micro text-ink-4">{x.fills.length}</span>
                </TabsTrigger>
              </TabsList>
            </div>

            <TabsContent value="balances" className="flex flex-col">
              <div className="flex flex-wrap items-center gap-3 border-b border-rule px-3 py-2">
                <label className="flex cursor-pointer items-center gap-2 font-sans text-micro text-ink-3">
                  <input
                    type="checkbox"
                    className="size-3.5 accent-[var(--color-control)]"
                    checked={hideZero}
                    onChange={(e) => setHideZero(e.target.checked)}
                    data-testid="portfolio-hide-zero"
                  />
                  Hide zero balances
                </label>
                <ActionButton
                  type="button"
                  className="ml-auto h-8 min-w-[96px] px-3 text-micro"
                  data-testid="portfolio-deposit"
                  onClick={() => openDeposit()}
                >
                  Deposit
                </ActionButton>
              </div>

              <ColumnHeads className={COLS}>
                <span>Asset</span>
                <span>Total balance</span>
                <span className="max-stack:hidden">Available</span>
                <span className="max-stack:hidden">In open orders</span>
                <span>Value</span>
                <span className="max-stack:hidden">Share</span>
                <span aria-hidden="true" className="max-stack:hidden" />
              </ColumnHeads>
              <Scroll className="min-h-14 flex-none">
                {!x.session && (
                  <div className="flex items-center justify-between gap-3 border-b border-rule px-3 py-2">
                    <p className="font-sans text-micro text-ink-4">Sign in to deposit and trade.</p>
                    <ActionButton
                      type="button"
                      className="h-8 shrink-0 px-3 text-micro"
                      data-testid="portfolio-try-guest"
                      onClick={() => void x.signInAsGuest()}
                    >
                      Try as guest
                    </ActionButton>
                  </div>
                )}
                {visibleRows.length === 0 ? (
                  <div className="flex flex-col items-center gap-3 p-6 text-center">
                    <p className="font-sans text-micro text-ink-4">No balances yet. Deposit to get started.</p>
                    <ActionButton
                      type="button"
                      className="h-9 px-4"
                      onClick={() => openDeposit()}
                    >
                      Deposit
                    </ActionButton>
                  </div>
                ) : (
                  visibleRows.map((row) => {
                    const dp = decimalsForAsset(row.asset, x.markets);
                    const quiet = row.total === 0n;
                    const share = row.sharePct ?? 0;
                    return (
                      <div
                        key={row.asset}
                        className={`group tnum grid min-h-9 items-center border-b border-rule px-2.5 hover:bg-row-hover ${COLS}`}
                        data-testid="portfolio-row"
                        data-asset={row.asset}
                      >
                        <span className="flex items-center gap-2.5 text-left">
                          <AssetIcon asset={row.asset} />
                          <span className="flex min-w-0 flex-col leading-tight">
                            <span className={`truncate font-sans text-label ${quiet ? "text-ink-4" : "text-ink-2"}`}>
                              {assetFullName(row.asset)}
                            </span>
                            <span className="font-sans text-micro text-ink-4">{row.asset}</span>
                          </span>
                        </span>
                        <AmountCell
                          atoms={row.total}
                          decimals={dp}
                          valueAtoms={row.value}
                          quiet={quiet}
                          compact={compactPlaces(row.asset, x.markets)}
                        />
                        <div className="max-stack:hidden">
                          <AmountCell
                            atoms={row.available}
                            decimals={dp}
                            valueAtoms={row.availableValue}
                            quiet={quiet}
                            compact={compactPlaces(row.asset, x.markets)}
                          />
                        </div>
                        <div className="max-stack:hidden">
                          <AmountCell
                            atoms={row.locked}
                            decimals={dp}
                            valueAtoms={row.lockedValue}
                            quiet={quiet}
                            compact={compactPlaces(row.asset, x.markets)}
                          />
                        </div>
                        <div className="flex flex-col items-end leading-tight">
                          <span className={`max-stack:hidden ${quiet ? "text-ink-4" : undefined}`}>
                            {row.total === 0n ? (
                              <Num atoms={0n} decimals={6n} places={2} />
                            ) : row.value === null ? (
                              <span className="text-ink-4">-</span>
                            ) : (
                              <Num atoms={row.value} decimals={6n} places={2} />
                            )}
                          </span>
                          <span className="font-sans text-micro">
                            <DollarValue
                              atoms={row.value ?? (row.total === 0n ? 0n : null)}
                              muted={quiet}
                            />
                          </span>
                        </div>
                        <span className="flex min-w-0 items-center justify-end gap-1.5 overflow-hidden max-stack:hidden">
                          <span className="relative h-[3px] w-8 shrink-0 bg-rule group-hover:hidden">
                            <i
                              className="absolute inset-y-0 left-0 rounded-full"
                              style={{
                                width: `${Math.min(share, 100)}%`,
                                backgroundColor: assetColor(row.asset),
                              }}
                            />
                          </span>
                          <span
                            className={`shrink-0 tabular-nums group-hover:hidden ${quiet ? "text-ink-4" : "text-ink-3"}`}
                          >
                            {quiet || row.sharePct !== null
                              ? `${(row.sharePct ?? 0).toFixed(quiet ? 0 : 1)}%`
                              : "-"}
                          </span>
                        </span>
                        <span className="overflow-hidden text-right max-stack:hidden">
                          <button
                            type="button"
                            className="hidden cursor-pointer whitespace-nowrap rounded-control px-1 py-0.5 font-sans text-micro text-control hover:bg-field group-hover:inline focus:inline"
                            onClick={() => openDeposit(row.asset)}
                            data-testid={`portfolio-row-deposit-${row.asset}`}
                          >
                            Deposit
                          </button>
                        </span>
                      </div>
                    );
                  })
                )}
              </Scroll>
            </TabsContent>

            <TabsContent value="orders" className="flex min-h-0 flex-1 flex-col">
              <OpenOrders orders={x.openOrders} markets={x.markets} onCancel={(id) => void x.cancel(id)} />
            </TabsContent>

            <TabsContent value="fills" className="flex min-h-0 flex-1 flex-col">
              <MyFills fills={x.fills} markets={x.markets} />
            </TabsContent>
          </Tabs>
        </div>
      </div>

      {depositOpen && (
        <DepositDialog
          markets={x.markets}
          signedIn={x.session !== null}
          initialAsset={depositAsset}
          onGuest={() => void x.signInAsGuest()}
          onDeposit={x.credit}
          onClose={() => setDepositOpen(false)}
        />
      )}
    </>
  );
}
