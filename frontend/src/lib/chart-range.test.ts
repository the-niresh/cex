import { describe, expect, it } from "vitest";
import {
  CHART_RIGHT_OFFSET,
  CHART_VISIBLE_BARS,
  chartViewKey,
  initialVisibleRange,
  planChartSeriesUpdate,
} from "./chart-range";

describe("chartViewKey", () => {
  it("combines symbol and interval", () => {
    expect(chartViewKey("BTC_USDT", "1m")).toBe("BTC_USDT:1m");
  });
});

describe("initialVisibleRange", () => {
  it("shows the last hundred bars with a right offset", () => {
    expect(initialVisibleRange(250)).toEqual({
      from: 150,
      to: 249 + CHART_RIGHT_OFFSET,
    });
  });

  it("uses every bar when fewer than the default window", () => {
    expect(initialVisibleRange(12)).toEqual({
      from: 0,
      to: 11 + CHART_RIGHT_OFFSET,
    });
  });

  it("returns null for an empty series", () => {
    expect(initialVisibleRange(0)).toBeNull();
  });

  it("caps at CHART_VISIBLE_BARS", () => {
    const range = initialVisibleRange(CHART_VISIBLE_BARS);
    expect(range).toEqual({ from: 0, to: CHART_VISIBLE_BARS - 1 + CHART_RIGHT_OFFSET });
  });
});

describe("planChartSeriesUpdate", () => {
  const key = chartViewKey("BTC_USDT", "1m");
  const times = [1, 2, 3, 4, 5];

  it("replaces data on first load", () => {
    expect(planChartSeriesUpdate(key, null, [], times)).toEqual({
      kind: "full",
      range: initialVisibleRange(times.length),
    });
  });

  it("replaces data when the market or interval changes", () => {
    expect(planChartSeriesUpdate("ETH_USDT:1m", key, times, times)).toEqual({
      kind: "full",
      range: initialVisibleRange(times.length),
    });
  });

  it("updates incrementally when the prefix is unchanged", () => {
    expect(planChartSeriesUpdate(key, key, times, times)).toEqual({
      kind: "incremental",
      fromIndex: 4,
    });
  });

  it("appends incrementally when a new bar arrives", () => {
    expect(planChartSeriesUpdate(key, key, times, [...times, 6])).toEqual({
      kind: "incremental",
      fromIndex: 4,
    });
  });

  it("keeps the visible range when the history is rewritten", () => {
    expect(planChartSeriesUpdate(key, key, times, [2, 3, 4, 5, 6])).toEqual({
      kind: "full-keep-range",
    });
  });
});
