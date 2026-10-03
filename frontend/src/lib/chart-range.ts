/** How many bars the chart shows on first load or after market / interval change. */
export const CHART_VISIBLE_BARS = 100;

/** Matches lightweight-charts `timeScale.rightOffset`. */
export const CHART_RIGHT_OFFSET = 4;

export type ChartViewKey = string;

export function chartViewKey(symbol: string, interval: string): ChartViewKey {
  return `${symbol}:${interval}`;
}

/** Logical range for the newest `barCount` candles. */
export function initialVisibleRange(barCount: number): { from: number; to: number } | null {
  if (barCount <= 0) return null;
  const visibleBars = Math.min(CHART_VISIBLE_BARS, barCount);
  return {
    from: barCount - visibleBars,
    to: barCount - 1 + CHART_RIGHT_OFFSET,
  };
}

export type ChartSeriesPlan =
  | { kind: "full"; range: { from: number; to: number } | null }
  | { kind: "incremental"; fromIndex: number }
  | { kind: "full-keep-range" };

/**
 * Decide how to push candle data into lightweight-charts.
 *
 * On a refresh the visible range must stay put; only the first load and a
 * market or interval change may reposition the view.
 */
export function planChartSeriesUpdate(
  viewKey: ChartViewKey,
  prevViewKey: ChartViewKey | null,
  prevTimes: readonly number[],
  nextTimes: readonly number[],
): ChartSeriesPlan {
  if (viewKey !== prevViewKey || prevTimes.length === 0) {
    return { kind: "full", range: initialVisibleRange(nextTimes.length) };
  }

  if (nextTimes.length === 0) {
    return { kind: "full", range: null };
  }

  const sharedPrefix =
    prevTimes.length <= nextTimes.length &&
    prevTimes.every((time, index) => time === nextTimes[index]);

  if (sharedPrefix && nextTimes.length >= prevTimes.length) {
    return { kind: "incremental", fromIndex: Math.max(0, prevTimes.length - 1) };
  }

  return { kind: "full-keep-range" };
}
