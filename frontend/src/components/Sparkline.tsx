interface Props {
  /** Oldest first. Fewer than two points draws nothing — a line needs a span. */
  values: number[];
  /** Drawn in this colour; the fill under it is the same hue at low alpha. */
  stroke: string;
  width?: number;
  height?: number;
  label: string;
}

/**
 * A plain inline SVG sparkline. No charting library: this is a polyline over
 * fifty numbers, and pulling in a dependency to draw one would cost more than
 * it saves.
 *
 * Scaled to its own max rather than a fixed ceiling, so the shape of recent
 * variation stays visible whether the numbers are microseconds or seconds.
 * That means the height is *relative* — which is why the strip always prints
 * the p50 and p99 next to it. A sparkline alone would show shape and hide
 * magnitude, and magnitude is the point.
 */
export function Sparkline({ values, stroke, width = 132, height = 22, label }: Props) {
  if (values.length < 2) {
    return (
      <svg
        width={width}
        height={height}
        role="img"
        aria-label={`${label}: not enough samples yet`}
        className="overflow-visible"
      />
    );
  }

  const max = Math.max(...values);
  const span = values.length - 1;
  // A flat series would divide by zero; it draws along the baseline instead.
  const y = (v: number) => (max > 0 ? height - (v / max) * (height - 2) - 1 : height - 1);
  const x = (i: number) => (i / span) * width;

  const line = values.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
  const area = `${x(0)},${height} ${line} ${x(span)},${height}`;

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={`${label}: last ${values.length} samples`}
      preserveAspectRatio="none"
    >
      <polygon points={area} fill={stroke} fillOpacity={0.14} />
      <polyline
        points={line}
        fill="none"
        stroke={stroke}
        strokeWidth={1}
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
