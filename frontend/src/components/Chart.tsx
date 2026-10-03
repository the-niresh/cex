import { useEffect, useRef } from "react";
import {
  CandlestickSeries,
  HistogramSeries,
  TickMarkType,
  createChart,
  isBusinessDay,
  isUTCTimestamp,
  type AutoscaleInfo,
  type IChartApi,
  type ISeriesApi,
  type Time,
  type UTCTimestamp,
} from "lightweight-charts";
import { autoscaleInfoForVisibleBars } from "@/lib/chart-autoscale";
import { cn } from "@/lib/utils";
import { chartViewKey, planChartSeriesUpdate } from "../lib/chart-range";
import { withAlpha } from "../lib/color";
import { decimalsForStep } from "../lib/num";
import type { Candle, Interval, Market } from "../lib/types";
import { Empty, Meta, Panel, PanelHead, PanelTitle } from "./ui/panel";

const INTERVALS: Interval[] = ["1m", "5m", "15m", "1h", "4h", "1d"];

function tickTime(time: Time): Date {
  if (isUTCTimestamp(time)) return new Date(time * 1000);
  if (isBusinessDay(time)) return new Date(time.year, time.month - 1, time.day);
  return new Date(time);
}

/** One format per tick weight so the axis never ends on a lone month name. */
function formatChartTick(time: Time, tickMarkType: TickMarkType): string {
  const date = tickTime(time);
  const month = date.toLocaleString("en-US", { month: "short" });
  const day = date.getDate();
  const hours = date.getHours().toString().padStart(2, "0");
  const mins = date.getMinutes().toString().padStart(2, "0");

  switch (tickMarkType) {
    case TickMarkType.Year:
      return String(date.getFullYear());
    case TickMarkType.Month:
      return `${month} ${day}`;
    case TickMarkType.DayOfMonth:
      return `${month} ${day}`;
    case TickMarkType.Time:
      return `${hours}:${mins}`;
    case TickMarkType.TimeWithSeconds:
      return `${hours}:${mins}:${date.getSeconds().toString().padStart(2, "0")}`;
    default:
      return `${month} ${day}`;
  }
}

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
  const viewKeyRef = useRef<string | null>(null);
  const candleTimesRef = useRef<number[]>([]);
  const pricePointsRef = useRef<
    Array<{ time: UTCTimestamp; open: number; high: number; low: number; close: number }>
  >([]);

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
        tickMarkFormatter: (time: Time, tickMarkType: TickMarkType) =>
          formatChartTick(time, tickMarkType),
        rightOffset: 4,
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
      autoscaleInfoProvider: (original: () => AutoscaleInfo | null) => {
        const visible = chart.timeScale().getVisibleLogicalRange();
        const points = pricePointsRef.current;
        if (!visible || points.length === 0) return original();

        const from = Math.max(0, Math.ceil(visible.from));
        const to = Math.min(points.length - 1, Math.floor(visible.to));
        if (from > to) return original();

        return autoscaleInfoForVisibleBars(points.slice(from, to + 1), original);
      },
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
    // Keep the price tag off the time-axis ticks, especially on the 300px phone
    // panel where the volume pane leaves little room at the bottom.
    chart.priceScale("right").applyOptions({ scaleMargins: { top: 0.05, bottom: 0.28 } });

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
    const chart = chartRef.current;
    const price = priceRef.current;
    const volume = volumeRef.current;
    if (!chart || !price || !volume || !market) return;

    // Volume is context, not the subject, so it is drawn at a third strength.
    // ⚠️ `rgba`, not `color-mix` — see the note on `withAlpha`.
    const css = getComputedStyle(document.documentElement);
    const ghost = (name: string) => withAlpha(css.getPropertyValue(name), 0.32);

    const quoteUnit = Number(10n ** market.quote_decimals);
    const baseUnit = Number(10n ** market.base_decimals);
    const toPrice = (atoms: bigint) => Number(atoms) / quoteUnit;

    const pricePoints = candles.map((c) => ({
      time: (Number(c.time_ms) / 1000) as UTCTimestamp,
      open: toPrice(c.open),
      high: toPrice(c.high),
      low: toPrice(c.low),
      close: toPrice(c.close),
    }));
    pricePointsRef.current = pricePoints;

    const volumePoints = candles.map((c) => ({
      time: (Number(c.time_ms) / 1000) as UTCTimestamp,
      value: Number(c.volume) / baseUnit,
      color: c.close >= c.open ? ghost("--color-buy") : ghost("--color-sell"),
    }));

    const viewKey = chartViewKey(market.symbol, interval);
    const nextTimes = pricePoints.map((p) => p.time);
    const plan = planChartSeriesUpdate(
      viewKey,
      viewKeyRef.current,
      candleTimesRef.current,
      nextTimes,
    );

    if (plan.kind === "full") {
      price.setData(pricePoints);
      volume.setData(volumePoints);
      if (plan.range) chart.timeScale().setVisibleLogicalRange(plan.range);
      viewKeyRef.current = viewKey;
    } else if (plan.kind === "incremental") {
      for (let i = plan.fromIndex; i < pricePoints.length; i++) {
        price.update(pricePoints[i]);
        volume.update(volumePoints[i]);
      }
    } else {
      const range = chart.timeScale().getVisibleLogicalRange();
      price.setData(pricePoints);
      volume.setData(volumePoints);
      if (range) chart.timeScale().setVisibleLogicalRange(range);
    }

    candleTimesRef.current = nextTimes;

    price.applyOptions({
      priceFormat: {
        type: "price",
        precision: decimalsForStep(market.tick_size, market.quote_decimals),
        minMove: Number(market.tick_size) / quoteUnit,
      },
    });
  }, [candles, market, interval]);

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
        className="relative min-h-0 flex-1 bg-transparent group-data-[degraded=true]/screen:saturate-[.15] group-data-[degraded=true]/screen:brightness-[.62]"
        ref={containerRef}
      >
        {candles.length === 0 && <Empty>no trades in this window yet</Empty>}
      </div>
    </Panel>
  );
}
