import { useEffect, useRef } from "react";
import {
  CandlestickSeries,
  HistogramSeries,
  createChart,
  type IChartApi,
  type ISeriesApi,
  type UTCTimestamp,
} from "lightweight-charts";
import { cn } from "@/lib/utils";
import { withAlpha } from "../lib/color";
import { decimalsForStep } from "../lib/num";
import type { Candle, Interval, Market } from "../lib/types";
import { Empty, Meta, Panel, PanelHead, PanelTitle } from "./ui/panel";

const INTERVALS: Interval[] = ["1m", "5m", "15m", "1h", "4h", "1d"];

interface Props {
  market: Market | null;
  candles: Candle[];
  interval: Interval;
  onInterval(interval: Interval): void;
}

/**
 * Candles, drawn by lightweight-charts from this exchange's own fills.
 *
 * The library wants floats, so atomic integers are divided *here and only
 * here*, at the very edge, for pixels. Nothing downstream of this conversion
 * may ever price, value or settle anything — see the candle endpoint's note in
 * the README.
 */
export function Chart({ market, candles, interval, onInterval }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const priceRef = useRef<ISeriesApi<"Candlestick"> | null>(null);
  const volumeRef = useRef<ISeriesApi<"Histogram"> | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    // ⚠️ lightweight-charts paints into a canvas, and canvas colours cannot be
    // `var(--x)` — the string is not a colour and the library silently keeps
    // whatever was there. So the tokens are resolved to real values here, once,
    // rather than hardcoded a second time in this file.
    const css = getComputedStyle(document.documentElement);
    const token = (name: string) => css.getPropertyValue(name).trim();

    const chart = createChart(container, {
      layout: {
        background: { color: "transparent" },
        textColor: token("--color-ink-3"),
        fontFamily: "'IBM Plex Mono', ui-monospace, monospace",
        fontSize: 11,
      },
      grid: {
        vertLines: { color: token("--color-rule") },
        horzLines: { color: token("--color-rule") },
      },
      rightPriceScale: { borderColor: token("--color-rule-hi") },
      timeScale: {
        borderColor: token("--color-rule-hi"),
        timeVisible: true,
        secondsVisible: false,
        // `fitContent` below spreads whatever bars exist across the panel. On a
        // young market that is a dozen bars over 900px, and a candle 100px wide
        // stops reading as a candle. Capping the spacing keeps them the shape
        // of candles; the panel is simply not full yet, which is the truth.
        maxBarSpacing: 18,
      },
      crosshair: {
        vertLine: { color: token("--color-ink-3"), width: 1, style: 2, labelBackgroundColor: token("--color-panel-hi") },
        horzLine: { color: token("--color-ink-3"), width: 1, style: 2, labelBackgroundColor: token("--color-panel-hi") },
      },
      autoSize: true,
    });

    const price = chart.addSeries(CandlestickSeries, {
      upColor: token("--color-buy"),
      downColor: token("--color-sell"),
      borderUpColor: token("--color-buy"),
      borderDownColor: token("--color-sell"),
      wickUpColor: token("--color-buy"),
      wickDownColor: token("--color-sell"),
    });

    const volume = chart.addSeries(HistogramSeries, {
      priceFormat: { type: "volume" },
      priceScaleId: "volume",
      // Its own scale already sits under the candles; a last-value tag and a
      // price line on the right axis would only compete with the price's.
      lastValueVisible: false,
      priceLineVisible: false,
    });
    // Volume is context, not the subject: it lives in the bottom fifth.
    chart.priceScale("volume").applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });

    chartRef.current = chart;
    priceRef.current = price;
    volumeRef.current = volume;

    return () => {
      chart.remove();
      chartRef.current = null;
      priceRef.current = null;
      volumeRef.current = null;
    };
  }, []);

  useEffect(() => {
    const price = priceRef.current;
    const volume = volumeRef.current;
    if (!price || !volume || !market) return;

    // Volume is context, not the subject, so it is drawn at a third strength.
    // ⚠️ `rgba`, not `color-mix` — see the note on `withAlpha`.
    const css = getComputedStyle(document.documentElement);
    const ghost = (name: string) => withAlpha(css.getPropertyValue(name), 0.32);

    const quoteUnit = Number(10n ** market.quote_decimals);
    const baseUnit = Number(10n ** market.base_decimals);
    const toPrice = (atoms: bigint) => Number(atoms) / quoteUnit;

    price.setData(
      candles.map((c) => ({
        time: (Number(c.time_ms) / 1000) as UTCTimestamp,
        open: toPrice(c.open),
        high: toPrice(c.high),
        low: toPrice(c.low),
        close: toPrice(c.close),
      })),
    );

    volume.setData(
      candles.map((c) => ({
        time: (Number(c.time_ms) / 1000) as UTCTimestamp,
        value: Number(c.volume) / baseUnit,
        color: c.close >= c.open ? ghost("--color-buy") : ghost("--color-sell"),
      })),
    );

    price.applyOptions({
      priceFormat: {
        type: "price",
        precision: decimalsForStep(market.tick_size, market.quote_decimals),
        minMove: Number(market.tick_size) / quoteUnit,
      },
    });

    // Spread whatever bars exist across the panel. Without this the chart keeps
    // its default bar spacing and pins the series to the right edge, so a young
    // market — which is every market here that is not BTC_USDT — draws a dozen
    // candles in the last tenth of the width and leaves the rest blank, looking
    // broken rather than new.
    chartRef.current?.timeScale().fitContent();
  }, [candles, market]);

  return (
    // ⚠️ A fixed height in the stacked layout, not a floor. The rows there are
    // auto-height, and lightweight-charts sizes its canvas from the container it
    // is given — so a container that sizes itself from its content grows every
    // time the observer fires, and the panel ran to a thousand pixels on a
    // phone. The old rule set a floor and no ceiling, and did the same.
    <Panel className="max-stack:h-[300px]" data-testid="chart-panel">
      <PanelHead>
        <PanelTitle>Chart</PanelTitle>
        <Meta>
          {market?.symbol ?? "—"} · <b className="tnum font-medium text-ink-2">{interval}</b> ·{" "}
          <span className="tnum">{candles.length}</span> bars
        </Meta>
      </PanelHead>

      {/* 32px, matching the reference's timeframe row. The reference separates
          buttons with spacing, not the underline and border-r dividers the old
          row used — see PanelTabs for the same pill treatment. */}
      <div className="flex h-8 flex-none items-center gap-1 border-b border-rule bg-panel px-2 py-1">
        {INTERVALS.map((option) => (
          <button
            key={option}
            type="button"
            aria-selected={option === interval}
            onClick={() => onInterval(option)}
            className={cn(
              "flex min-h-6 cursor-pointer items-center rounded-control px-2.5",
              "font-sans text-label transition-colors",
              option === interval ? "bg-field text-ink" : "text-ink-4 hover:text-ink-2",
            )}
          >
            {option === "1d" ? "1D" : option}
          </button>
        ))}
      </div>

      {/* lightweight-charts paints into this and owns its own canvas sizing.
          The chart is data too, so it goes flat and grey when the feed does. */}
      <div
        className="relative min-h-0 flex-1 bg-transparent group-data-[stale=true]/screen:saturate-[.15] group-data-[stale=true]/screen:brightness-[.62]"
        ref={containerRef}
      >
        {candles.length === 0 && <Empty>no trades in this window yet</Empty>}
      </div>
    </Panel>
  );
}
