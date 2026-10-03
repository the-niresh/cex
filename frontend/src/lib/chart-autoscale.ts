import type { AutoscaleInfo } from "lightweight-charts";

/** One candle's prices after the bigint-to-float edge conversion in Chart.tsx. */
export type ChartOhlc = {
  low: number;
  high: number;
  close: number;
};

export type ChartPriceRange = {
  minValue: number;
  maxValue: number;
};

const P_LOW = 0.02;
const P_HIGH = 0.98;
const DEFAULT_PADDING_FRACTION = 0.03;

function percentile(sorted: readonly number[], q: number): number {
  if (sorted.length === 0) return NaN;
  if (sorted.length === 1) return sorted[0]!;

  const position = q * (sorted.length - 1);
  const lower = Math.floor(position);
  const upper = Math.ceil(position);
  if (lower === upper) return sorted[lower]!;

  const weight = position - lower;
  return sorted[lower]! * (1 - weight) + sorted[upper]! * weight;
}

/** Min low and max high across the bars, matching default candle autoscale. */
export function naivePriceRange(bars: readonly ChartOhlc[]): ChartPriceRange | null {
  if (bars.length === 0) return null;

  let minValue = Infinity;
  let maxValue = -Infinity;
  for (const bar of bars) {
    minValue = Math.min(minValue, bar.low);
    maxValue = Math.max(maxValue, bar.high);
  }
  return { minValue, maxValue };
}

/**
 * Robust core range before padding: 2nd percentile of lows, 98th of highs, with the
 * last close kept inside so the last-value label stays on screen.
 */
export function robustCorePriceRange(bars: readonly ChartOhlc[]): ChartPriceRange | null {
  if (bars.length === 0) return null;

  const lows = [...bars.map((bar) => bar.low)].sort((a, b) => a - b);
  const highs = [...bars.map((bar) => bar.high)].sort((a, b) => a - b);

  let minValue = percentile(lows, P_LOW);
  let maxValue = percentile(highs, P_HIGH);

  if (minValue > maxValue) {
    minValue = lows[0]!;
    maxValue = highs[highs.length - 1]!;
  }

  const lastClose = bars[bars.length - 1]!.close;
  return {
    minValue: Math.min(minValue, lastClose),
    maxValue: Math.max(maxValue, lastClose),
  };
}

/** Visible-bar autoscale with a small padding band. */
export function robustVisiblePriceRange(
  bars: readonly ChartOhlc[],
  paddingFraction = DEFAULT_PADDING_FRACTION,
): ChartPriceRange | null {
  const core = robustCorePriceRange(bars);
  if (!core) return null;

  const span = core.maxValue - core.minValue;
  const pad =
    span === 0 ? Math.max(Math.abs(core.minValue) * 0.001, 1e-8) : span * paddingFraction;

  return {
    minValue: core.minValue - pad,
    maxValue: core.maxValue + pad,
  };
}

export function priceRangesEqual(
  a: ChartPriceRange,
  b: ChartPriceRange,
  epsilon = 1e-9,
): boolean {
  return (
    Math.abs(a.minValue - b.minValue) <= epsilon && Math.abs(a.maxValue - b.maxValue) <= epsilon
  );
}

/**
 * Scale to visible bars with outlier trimming. When every bar is inside the robust
 * core range, defer to the library's default autoscale so behaviour is unchanged.
 */
export function autoscaleInfoForVisibleBars(
  bars: readonly ChartOhlc[],
  original: () => AutoscaleInfo | null,
  paddingFraction = DEFAULT_PADDING_FRACTION,
): AutoscaleInfo | null {
  if (bars.length === 0) return original();

  const naive = naivePriceRange(bars);
  const core = robustCorePriceRange(bars);
  if (naive && core) {
    const naiveSpan = naive.maxValue - naive.minValue;
    const coreSpan = core.maxValue - core.minValue;
    if (naiveSpan === 0 || coreSpan / naiveSpan >= 0.98) {
      return original();
    }
  }

  const robust = robustVisiblePriceRange(bars, paddingFraction);
  if (!robust) return original();

  const base = original();
  return {
    priceRange: robust,
    margins: base?.margins,
  };
}
