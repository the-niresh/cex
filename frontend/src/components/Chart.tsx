import { useEffect, useRef } from "react";
import {
  CandlestickSeries,
  HistogramSeries,
  createChart,
  type IChartApi,
  type ISeriesApi,
  type UTCTimestamp,
} from "lightweight-charts";
import { decimalsForStep } from "../lib/num";
import type { Candle, Interval, Market } from "../lib/types";

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

    const chart = createChart(container, {
      layout: {
        background: { color: "transparent" },
        textColor: "#818d9c",
        fontFamily: "'IBM Plex Mono', ui-monospace, monospace",
        fontSize: 10,
      },
      grid: {
        vertLines: { color: "#171b21" },
        horzLines: { color: "#171b21" },
      },
      rightPriceScale: { borderColor: "#232a33" },
      timeScale: {
        borderColor: "#232a33",
        timeVisible: true,
        secondsVisible: false,
        // `fitContent` below spreads whatever bars exist across the panel. On a
        // young market that is a dozen bars over 900px, and a candle 100px wide
        // stops reading as a candle. Capping the spacing keeps them the shape
        // of candles; the panel is simply not full yet, which is the truth.
        maxBarSpacing: 18,
      },
      crosshair: {
        vertLine: { color: "#818d9c", width: 1, style: 2, labelBackgroundColor: "#101317" },
        horzLine: { color: "#818d9c", width: 1, style: 2, labelBackgroundColor: "#101317" },
      },
      autoSize: true,
    });

    const price = chart.addSeries(CandlestickSeries, {
      upColor: "#17b98a",
      downColor: "#e2504f",
      borderUpColor: "#17b98a",
      borderDownColor: "#e2504f",
      wickUpColor: "#17b98a",
      wickDownColor: "#e2504f",
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
        color: c.close >= c.open ? "rgba(23,185,138,.32)" : "rgba(226,80,79,.32)",
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
    <section className="panel chart">
      <div className="phead">
        <h2>Chart</h2>
        <span className="meta">
          {market?.symbol ?? "—"} · <b>{interval}</b> · {candles.length} bars
        </span>
      </div>
      <div className="tfs">
        {INTERVALS.map((option) => (
          <button
            key={option}
            aria-selected={option === interval}
            onClick={() => onInterval(option)}
          >
            {option === "1d" ? "1D" : option}
          </button>
        ))}
      </div>
      <div className="plot live" ref={containerRef}>
        {candles.length === 0 && <div className="empty">no trades in this window yet</div>}
      </div>
    </section>
  );
}
