import { useEffect, useMemo, useRef } from "react";
import { AreaSeries, createChart, type IChartApi, type ISeriesApi, type UTCTimestamp } from "lightweight-charts";
import { withAlpha } from "../lib/color";
import { assetValue } from "../lib/portfolio";
import type { Balance, Candle, Market } from "../lib/types";

export interface ValuePoint {
  time: UTCTimestamp;
  value: number;
}

interface Props {
  balances: Balance[];
  candlesBySymbol: Map<string, Candle[]>;
  markets: Market[];
}

/**
 * Build a 24-point series: current holdings valued at each hour's close.
 * Not a real account history; labelled plainly on screen.
 */
const QUOTE_UNIT = 1_000_000;

function holdingsMap(balances: Balance[]): Map<string, bigint> {
  const holdings = new Map<string, bigint>();
  for (const b of balances) {
    const amount = b.available + b.locked;
    if (amount > 0n) holdings.set(b.asset, amount);
  }
  return holdings;
}

function valueAtBucket(
  holdings: Map<string, bigint>,
  markets: Market[],
  candlesBySymbol: Map<string, Candle[]>,
  timeMs: bigint | null,
): number {
  let total = 0n;
  for (const [asset, amount] of holdings) {
    if (asset === "USDT") {
      total += amount;
      continue;
    }
    const market = markets.find((m) => m.base === asset);
    if (!market) continue;
    const candles = candlesBySymbol.get(market.symbol);
    const close =
      timeMs === null
        ? (candles?.at(-1)?.close ?? null)
        : (candles?.find((c) => c.time_ms === timeMs)?.close ?? null);
    const priced = assetValue(amount, asset, close, markets);
    if (priced !== null) total += priced;
  }
  return Number(total) / QUOTE_UNIT;
}

export function buildHoldingsValueSeries(
  balances: Balance[],
  candlesBySymbol: Map<string, Candle[]>,
  markets: Market[],
): ValuePoint[] {
  const holdings = holdingsMap(balances);
  const timeline = candlesBySymbol.values().next().value ?? [];
  const now = Math.floor(Date.now() / 1000);

  if (timeline.length === 0) {
    const flat = valueAtBucket(holdings, markets, candlesBySymbol, null);
    return Array.from({ length: 24 }, (_, i) => ({
      time: (now - (23 - i) * 3600) as UTCTimestamp,
      value: flat,
    }));
  }

  return timeline.map((bucket) => ({
    time: (Number(bucket.time_ms) / 1000) as UTCTimestamp,
    value: valueAtBucket(holdings, markets, candlesBySymbol, bucket.time_ms),
  }));
}

export function PortfolioValueChart({ balances, candlesBySymbol, markets }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRef = useRef<ISeriesApi<"Area"> | null>(null);

  const points = useMemo(
    () => buildHoldingsValueSeries(balances, candlesBySymbol, markets),
    [balances, candlesBySymbol, markets],
  );

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const css = getComputedStyle(document.documentElement);
    const token = (name: string) => css.getPropertyValue(name).trim();

    const chart = createChart(container, {
      layout: {
        background: { color: "transparent" },
        textColor: token("--color-ink-4"),
        fontFamily: "'IBM Plex Mono', ui-monospace, monospace",
        fontSize: 10,
      },
      grid: {
        vertLines: { visible: false },
        horzLines: { color: token("--color-rule") },
      },
      rightPriceScale: { borderVisible: false },
      timeScale: { borderVisible: false, visible: false },
      crosshair: { vertLine: { visible: false }, horzLine: { visible: false } },
      handleScroll: false,
      handleScale: false,
      autoSize: true,
    });

    const accent = withAlpha(token("--color-control"), 0.35);
    const line = token("--color-control");
    const series = chart.addSeries(AreaSeries, {
      lineColor: line,
      topColor: accent,
      bottomColor: "transparent",
      lineWidth: 2,
      priceLineVisible: false,
      lastValueVisible: false,
    });

    chartRef.current = chart;
    seriesRef.current = series;

    return () => {
      chart.remove();
      chartRef.current = null;
      seriesRef.current = null;
    };
  }, []);

  useEffect(() => {
    const series = seriesRef.current;
    const chart = chartRef.current;
    if (!series || !chart) return;
    series.setData(points);
    chart.timeScale().fitContent();
    chart.priceScale("right").applyOptions({ autoScale: true });
  }, [points]);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-1">
      <div className="font-sans text-micro text-ink-4">Value of current holdings, last 24h</div>
      <div ref={containerRef} className="min-h-[72px] flex-1" data-testid="portfolio-value-chart" />
    </div>
  );
}
