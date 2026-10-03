import { describe, expect, it } from "vitest";
import {
  autoscaleInfoForVisibleBars,
  naivePriceRange,
  priceRangesEqual,
  robustCorePriceRange,
  robustVisiblePriceRange,
  type ChartOhlc,
} from "./chart-autoscale";

const bar = (low: number, high: number, close = (low + high) / 2): ChartOhlc => ({
  low,
  high,
  close,
});

const normalBtcBar = () => bar(50_117, 50_120, 50_118.5);

describe("naivePriceRange", () => {
  it("uses the lowest low and highest high", () => {
    expect(naivePriceRange([bar(10, 12), bar(9, 11)])).toEqual({
      minValue: 9,
      maxValue: 12,
    });
  });
});

describe("robustCorePriceRange", () => {
  it("matches naive autoscale for normal bars in a tight band", () => {
    const bars = Array.from({ length: 50 }, normalBtcBar);
    const naive = naivePriceRange(bars);
    const core = robustCorePriceRange(bars);
    expect(naive).not.toBeNull();
    expect(core).not.toBeNull();
    expect(priceRangesEqual(naive!, core!)).toBe(true);
  });

  it("ignores a BTC_USDT spike wick while keeping tight normal bars readable", () => {
    const normal = Array.from({ length: 100 }, () => bar(50_118.2, 50_118.5, 50_118.35));
    const spike = bar(49_999, 50_123, 50_061);
    const bars = [...normal, spike];

    const naive = naivePriceRange(bars)!;
    const core = robustCorePriceRange(bars)!;

    expect(naive.minValue).toBe(49_999);
    expect(naive.maxValue).toBe(50_123);
    expect(core.minValue).toBeGreaterThanOrEqual(50_118.2);
    expect(core.maxValue).toBeLessThanOrEqual(50_118.5);
    expect(core.minValue).toBeLessThan(50_118.3);
    expect(core.maxValue).toBeGreaterThan(50_118.4);
  });

  it("ignores 2% low-price outliers so normal candles stay readable", () => {
    const normal = Array.from({ length: 98 }, normalBtcBar);
    const outliers = [bar(4, 6, 5), bar(5, 6, 5.5)];
    const bars = [...normal, ...outliers];

    const naive = naivePriceRange(bars)!;
    const core = robustCorePriceRange(bars)!;

    expect(naive.minValue).toBe(4);
    expect(naive.maxValue).toBe(50_120);
    expect(core.minValue).toBeGreaterThan(50_100);
    expect(core.maxValue).toBeLessThan(50_125);
    expect(core.minValue).toBeLessThan(50_118);
    expect(core.maxValue).toBeGreaterThan(50_119);
  });

  it("ignores 10% low-price outliers at 5 USDT", () => {
    const normal = Array.from({ length: 90 }, normalBtcBar);
    const outliers = Array.from({ length: 10 }, () => bar(4, 6, 5));
    const bars = [...normal, ...outliers];

    const core = robustCorePriceRange(bars)!;
    expect(core.minValue).toBeGreaterThan(50_100);
    expect(core.maxValue).toBeLessThan(50_125);
  });

  it("handles all-equal bars", () => {
    const bars = Array.from({ length: 20 }, () => bar(100, 100, 100));
    const core = robustCorePriceRange(bars)!;
    expect(core).toEqual({ minValue: 100, maxValue: 100 });
    const padded = robustVisiblePriceRange(bars)!;
    expect(padded.minValue).toBeLessThan(100);
    expect(padded.maxValue).toBeGreaterThan(100);
  });

  it("falls back to the plain visible range when fewer than ten bars remain", () => {
    const bars = [
      bar(10, 12),
      bar(100, 102),
      bar(11, 13),
      bar(10.5, 12.5),
      bar(11.5, 13.5),
      bar(10.2, 12.2),
      bar(11.2, 13.2),
      bar(10.8, 12.8),
    ];
    expect(robustCorePriceRange(bars)).toEqual(naivePriceRange(bars));
  });

  it("handles a single bar", () => {
    const only = bar(42, 44, 43);
    expect(robustCorePriceRange([only])).toEqual({ minValue: 42, maxValue: 44 });
    expect(naivePriceRange([only])).toEqual({ minValue: 42, maxValue: 44 });
  });
});

describe("autoscaleInfoForVisibleBars", () => {
  it("returns the library default when there are no outliers", () => {
    const bars = [bar(10, 12), bar(11, 13)];
    const original = () => ({
      priceRange: { minValue: 10, maxValue: 13 },
      margins: { above: 4, below: 4 },
    });

    expect(autoscaleInfoForVisibleBars(bars, original)).toEqual(original());
  });

  it("returns a trimmed range when an outlier is present", () => {
    const bars = [...Array.from({ length: 99 }, () => bar(100, 101)), bar(50, 200, 100.5)];
    const original = () => ({
      priceRange: { minValue: 50, maxValue: 200 },
      margins: { above: 2, below: 2 },
    });

    const info = autoscaleInfoForVisibleBars(bars, original);
    const range = info?.priceRange;
    expect(range).not.toBeNull();
    if (!range) return;
    expect(range.minValue).toBeGreaterThan(90);
    expect(range.maxValue).toBeLessThan(110);
    expect(info?.margins).toEqual({ above: 2, below: 2 });
  });
});
