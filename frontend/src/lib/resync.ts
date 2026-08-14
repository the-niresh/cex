/**
 * How long to wait before trying a resync again.
 *
 * Split out of `useExchange` so the rule can be read and tested on its own —
 * it is a decision about load, and getting it wrong is what turned a depth gap
 * into an outage once already.
 *
 * The rule in one line: **back off from retries that did not help, not from
 * gaps.** A gap is normal and the first answer to it should be immediate. A
 * resync that finishes with the book *still* stale is the signal worth reacting
 * to, because it means the read path is behind and asking again straight away
 * will only add to what it is already behind on.
 */

/** First wait after a resync that left the book stale. */
export const RESYNC_BACKOFF_MIN_MS = 250;

/** Ceiling on the wait. Past this, waiting longer only hides a broken feed. */
export const RESYNC_BACKOFF_MAX_MS = 8_000;

/** Widest random extra added to a wait, to keep separate tabs out of step. */
export const RESYNC_JITTER_MS = 250;

/**
 * The wait to use before the next resync.
 *
 * @param currentMs what the last round waited — 0 when the last one worked
 * @param bookStale whether the round that just finished left the book stale
 */
export function nextResyncBackoffMs(currentMs: number, bookStale: boolean): number {
  // It worked. Whatever we had been waiting no longer applies, and the next gap
  // deserves the same quick answer the first one got.
  if (!bookStale) return 0;

  const doubled = currentMs > 0 ? currentMs * 2 : RESYNC_BACKOFF_MIN_MS;
  return Math.min(Math.max(RESYNC_BACKOFF_MIN_MS, doubled), RESYNC_BACKOFF_MAX_MS);
}

/**
 * A backoff wait with jitter applied.
 *
 * `random` is a parameter rather than a call to `Math.random` so a test can say
 * what it returns. Zero stays zero: no wait means no wait, and adding jitter to
 * the healthy path would delay every ordinary gap for no reason.
 */
export function resyncWaitMs(backoffMs: number, random: number): number {
  if (backoffMs <= 0) return 0;
  return backoffMs + random * RESYNC_JITTER_MS;
}
