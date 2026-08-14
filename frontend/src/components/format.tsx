import { formatAtoms } from "../lib/num";

/**
 * A number with its decimals dimmed.
 *
 * The magnitude is what the eye needs first when scanning a ladder; the
 * precision matters only once you have stopped on a row. Splitting them is the
 * one typographic trick this screen leans on.
 */
export function Num({
  atoms,
  decimals,
  places,
  group = true,
}: {
  atoms: bigint;
  decimals: bigint;
  places?: number;
  group?: boolean;
}) {
  const text = formatAtoms(atoms, decimals, { places: places ?? undefined, group });
  const dot = text.indexOf(".");
  if (dot === -1) return <>{text}</>;
  return (
    <>
      {text.slice(0, dot)}
      <span className="text-ink-3">{text.slice(dot)}</span>
    </>
  );
}

/** `HH:MM:SS` in local time, from epoch milliseconds. */
export function clock(ms: bigint): string {
  return new Date(Number(ms)).toLocaleTimeString("en-GB", { hour12: false });
}

/** `HH:MM:SS.mmm`, for the tape where ordering within a second is visible. */
export function clockMs(ms: bigint): { time: string; millis: string } {
  const date = new Date(Number(ms));
  return {
    time: date.toLocaleTimeString("en-GB", { hour12: false }),
    millis: `.${String(date.getMilliseconds()).padStart(3, "0")}`,
  };
}

/** How long ago, compactly: `2m14s`. */
export function age(sinceMs: number): string {
  const seconds = Math.max(0, Math.floor((Date.now() - sinceMs) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m${String(seconds % 60).padStart(2, "0")}s`;
  return `${Math.floor(minutes / 60)}h${String(minutes % 60).padStart(2, "0")}m`;
}
