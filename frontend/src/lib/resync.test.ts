import { describe, expect, it } from "vitest";

import {
  RESYNC_BACKOFF_MAX_MS,
  RESYNC_BACKOFF_MIN_MS,
  RESYNC_JITTER_MS,
  nextResyncBackoffMs,
  resyncWaitMs,
} from "./resync";

describe("nextResyncBackoffMs", () => {
  it("waits for nothing after a resync that left the book healthy", () => {
    expect(nextResyncBackoffMs(0, false)).toBe(0);
  });

  it("forgets a long wait as soon as one round finally works", () => {
    // The point of the whole rule: recovery is immediate, not gradual. A market
    // that hiccuped for a minute should feel normal the instant it comes back.
    expect(nextResyncBackoffMs(RESYNC_BACKOFF_MAX_MS, false)).toBe(0);
  });

  it("starts at the minimum when a first round leaves the book stale", () => {
    expect(nextResyncBackoffMs(0, true)).toBe(RESYNC_BACKOFF_MIN_MS);
  });

  it("doubles while rounds keep failing to fix the book", () => {
    expect(nextResyncBackoffMs(RESYNC_BACKOFF_MIN_MS, true)).toBe(RESYNC_BACKOFF_MIN_MS * 2);
    expect(nextResyncBackoffMs(RESYNC_BACKOFF_MIN_MS * 2, true)).toBe(RESYNC_BACKOFF_MIN_MS * 4);
  });

  it("stops doubling at the ceiling", () => {
    expect(nextResyncBackoffMs(RESYNC_BACKOFF_MAX_MS, true)).toBe(RESYNC_BACKOFF_MAX_MS);
    expect(nextResyncBackoffMs(RESYNC_BACKOFF_MAX_MS * 4, true)).toBe(RESYNC_BACKOFF_MAX_MS);
  });

  it("reaches the ceiling in a handful of rounds, not hundreds", () => {
    // The storm this replaced ran at roughly sixteen resyncs a second. Walk the
    // rule forward and check it is throttling within a second or two of trouble
    // rather than after the damage is done.
    let wait = 0;
    let rounds = 0;
    while (wait < RESYNC_BACKOFF_MAX_MS && rounds < 100) {
      wait = nextResyncBackoffMs(wait, true);
      rounds += 1;
    }
    expect(rounds).toBeLessThanOrEqual(6);
  });
});

describe("resyncWaitMs", () => {
  it("does not delay the healthy path at all", () => {
    expect(resyncWaitMs(0, 0.99)).toBe(0);
  });

  it("adds no more than the jitter spread", () => {
    expect(resyncWaitMs(1_000, 0)).toBe(1_000);
    expect(resyncWaitMs(1_000, 1)).toBe(1_000 + RESYNC_JITTER_MS);
  });

  it("spreads two tabs that gapped at the same moment", () => {
    const one = resyncWaitMs(1_000, 0.1);
    const other = resyncWaitMs(1_000, 0.9);
    expect(one).not.toBe(other);
  });
});
