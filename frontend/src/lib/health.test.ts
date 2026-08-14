import { describe, expect, it } from "vitest";
import { SILENT_AFTER_MS, feedHealth } from "./health";

/**
 * The distinction these tests exist to defend: a book that cannot be trusted is
 * not the same thing as a market where nothing has happened.
 *
 * They were one flag once, and the consequence was that a perfectly good book
 * on a quiet market greyed itself out and switched the ticket off after eight
 * seconds — you could not place an order into a market that was working, which
 * is the worst possible time to refuse.
 */
describe("feedHealth", () => {
  const live = { bookStale: false, status: "live" as const, silentForMs: 0 };

  it("is healthy on a live feed that is keeping up", () => {
    expect(feedHealth(live)).toEqual({ degraded: false, fresh: true, reason: null });
  });

  it("does not degrade a market that has simply gone quiet", () => {
    const quiet = feedHealth({ ...live, silentForMs: 74_000 });
    // Nothing has printed, and the screen says so — but the book is still the
    // real book, so trading stays open.
    expect(quiet.degraded).toBe(false);
    expect(quiet.fresh).toBe(false);
    expect(quiet.reason).toBe("NO UPDATES FOR 74S");
  });

  it("degrades on a sequence gap, because the book is then a guess", () => {
    const gap = feedHealth({ ...live, bookStale: true });
    expect(gap.degraded).toBe(true);
    expect(gap.fresh).toBe(false);
    expect(gap.reason).toBe("STALE — SEQUENCE GAP, RESYNCING");
  });

  it("degrades whenever the socket is not live", () => {
    for (const status of ["connecting", "reconnecting", "closed"] as const) {
      const down = feedHealth({ ...live, status });
      expect(down.degraded, status).toBe(true);
      expect(down.reason, status).toBe(`STALE — SOCKET ${status.toUpperCase()}`);
    }
  });

  it("reports the gap ahead of the silence when both are true", () => {
    const both = feedHealth({ bookStale: true, status: "live", silentForMs: 90_000 });
    expect(both.reason).toBe("STALE — SEQUENCE GAP, RESYNCING");
  });

  it("holds off calling a market quiet until the threshold", () => {
    expect(feedHealth({ ...live, silentForMs: SILENT_AFTER_MS }).fresh).toBe(true);
    expect(feedHealth({ ...live, silentForMs: SILENT_AFTER_MS + 1 }).fresh).toBe(false);
  });

  it("treats a feed that has never delivered as fresh, not silent", () => {
    // Before the first depth message there is no gap to measure, and claiming
    // a market has gone quiet when it has not yet been heard from is a lie.
    expect(feedHealth({ ...live, silentForMs: null })).toEqual({
      degraded: false,
      fresh: true,
      reason: null,
    });
  });
});
