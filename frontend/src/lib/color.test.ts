import { describe, expect, it } from "vitest";
import { withAlpha } from "./color";

/**
 * ⚠️ This exists because lightweight-charts parses colours itself and rejects
 * anything CSS-modern. Handing it `color-mix(in oklab, #00c278 32%, transparent)`
 * throws "Failed to parse color" and the whole chart renders blank — no candles,
 * no grid, no axis. Nothing in the test suite caught it; it was visible only in
 * a screenshot.
 *
 * So the theme's hex tokens get turned into plain `rgba()` here before they go
 * anywhere near the canvas.
 */
describe("withAlpha", () => {
  it("turns a hex token into rgba the canvas can parse", () => {
    expect(withAlpha("#00c278", 0.32)).toBe("rgba(0, 194, 120, 0.32)");
    expect(withAlpha("#e45a59", 0.32)).toBe("rgba(228, 90, 89, 0.32)");
  });

  it("tolerates the whitespace getPropertyValue leaves behind", () => {
    expect(withAlpha("  #00c278 ", 0.5)).toBe("rgba(0, 194, 120, 0.5)");
  });

  it("accepts three-digit hex", () => {
    expect(withAlpha("#0c8", 1)).toBe("rgba(0, 204, 136, 1)");
  });

  it("refuses anything it cannot parse rather than emitting a broken colour", () => {
    // Returning a malformed string here is what blanked the chart. Throwing
    // fails loudly at the call site instead.
    expect(() => withAlpha("color-mix(in oklab, red 32%, transparent)", 0.3)).toThrow();
    expect(() => withAlpha("", 0.3)).toThrow();
  });
});
