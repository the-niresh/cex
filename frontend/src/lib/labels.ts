/**
 * Engine enum → what a person reads.
 *
 * Every enum on the wire is `SCREAMING_SNAKE_CASE` — `OPEN`, `PARTIALLY_FILLED`,
 * `LIMIT`, `TAKER`. The headings above them are sentence case, so leaving the
 * data shouting made the only loud thing on the screen the part nobody has to
 * read twice.
 *
 * This is one rule, not a lookup table: lower it, unscore it, capitalise the
 * first letter. A status the engine adds later therefore reads as words on the
 * day it ships, with nothing here to keep in sync.
 *
 * ⚠️ Deliberately *not* applied to `side`. BUY and SELL stay uppercase: side is
 * the one field scanned by shape rather than read, and it is the field a
 * mis-read costs the most.
 */
export function sentenceCase(wireValue: string): string {
  const words = wireValue.toLowerCase().replace(/_/g, " ").trim();
  if (words === "") return wireValue;
  return words.charAt(0).toUpperCase() + words.slice(1);
}
