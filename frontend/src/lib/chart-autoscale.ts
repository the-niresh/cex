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

const MIN_INLIER_BARS = 10;
const MAD_MULTIPLIER = 20;
const MEDIAN_BAND_FRACTION = 0.005;
const DEFAULT_PADDING_FRACTION = 0.03;

function median(values: readonly number[]): number {
  if (values.length === 0) return NaN;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  if (sorted.length % 2 === 1) return sorted[mid]!;
  return (sorted[mid - 1]! + sorted[mid]!) / 2;
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
 * Robust core range: median of closes, MAD of lows and highs from it, keep bars
 * within max(20 x MAD, 0.5% of median). Fall back to the plain visible range
 * when fewer than ten bars pass.
 */
export function robustCorePriceRange(bars: readonly ChartOhlc[]): ChartPriceRange | null {
  if (bars.length === 0) return null;

  const closeMedian = median(bars.map((bar) => bar.close));
  const deviations: number[] = [];
  for (const bar of bars) {
    deviations.push(Math.abs(bar.low - closeMedian));
    deviations.push(Math.abs(bar.high - closeMedian));
  }
  const mad = median(deviations);
  const band = Math.max(MAD_MULTIPLIER * mad, MEDIAN_BAND_FRACTION * Math.abs(closeMedian));

  const inliers = bars.filter(
    (bar) =>
      Math.abs(bar.low - closeMedian) <= band && Math.abs(bar.high - closeMedian) <= band,
  );

  if (inliers.length < MIN_INLIER_BARS) {
    return naivePriceRange(bars);
  }

  return naivePriceRange(inliers);
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
