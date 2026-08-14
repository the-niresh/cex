/**
 * Hex → `rgba()`, for colours that end up on a canvas.
 *
 * ⚠️ lightweight-charts parses colours with its own parser, not the browser's.
 * It understands hex, `rgb()` and `rgba()` and nothing newer — a `color-mix()`
 * makes it throw "Failed to parse color" and render an empty chart. The theme's
 * tokens are hex, so this is the one conversion needed to get a translucent
 * version of one onto the canvas.
 */
export function withAlpha(hex: string, alpha: number): string {
  const value = hex.trim().replace(/^#/, "");

  const full =
    value.length === 3
      ? value
          .split("")
          .map((c) => c + c)
          .join("")
      : value;

  if (!/^[0-9a-fA-F]{6}$/.test(full)) {
    // Loudly, rather than returning something the canvas will silently reject.
    throw new Error(`withAlpha: not a hex colour: ${JSON.stringify(hex)}`);
  }

  const r = parseInt(full.slice(0, 2), 16);
  const g = parseInt(full.slice(2, 4), 16);
  const b = parseInt(full.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
