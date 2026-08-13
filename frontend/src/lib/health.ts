import type { FeedStatus } from "./feed";

/**
 * How long a market may go without a depth update before the screen says so.
 *
 * This is a "nothing has happened lately" threshold, not a "something is
 * wrong" one — see `feedHealth`. Eight seconds is short on purpose: the point
 * is to answer "am I looking at a still picture?" quickly.
 */
export const SILENT_AFTER_MS = 8_000;

export interface FeedInputs {
  /** The book missed a sequence and is being refetched. */
  bookStale: boolean;
  status: FeedStatus;
  /** Since the last depth update, or `null` before the first one arrives. */
  silentForMs: number | null;
}

export interface FeedHealth {
  /**
   * The book cannot be trusted: the socket is down, or a sequence was missed
   * and what is on screen is a guess. This is the only condition that may stop
   * someone trading — pricing off a book you know is wrong is worse than not
   * trading at all.
   */
  degraded: boolean;
  /** An update arrived recently enough that the screen is a live picture. */
  fresh: boolean;
  /** Why the picture is not live, in words, or `null` when it is. */
  reason: string | null;
}

/**
 * Two questions the screen used to conflate into one.
 *
 * "Can I trust this book?" and "has anything happened lately?" have different
 * answers and deserve different consequences. A quiet market is not a broken
 * one: the book is correct, it simply has not changed, and switching the ticket
 * off because nobody traded for eight seconds locks people out of a market that
 * is working perfectly well.
 */
export function feedHealth({ bookStale, status, silentForMs }: FeedInputs): FeedHealth {
  const degraded = bookStale || status !== "live";
  const silent = silentForMs !== null && silentForMs > SILENT_AFTER_MS;

  // A gap is the more serious of the two and names itself first: it says what
  // is wrong, where the silence only says what has not happened.
  const reason = bookStale
    ? "STALE — SEQUENCE GAP, RESYNCING"
    : status !== "live"
      ? `STALE — SOCKET ${status.toUpperCase()}`
      : silent
        ? `NO UPDATES FOR ${Math.floor((silentForMs ?? 0) / 1000)}S`
        : null;

  return { degraded, fresh: !degraded && !silent, reason };
}
