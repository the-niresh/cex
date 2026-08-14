//! The shape of a demo market: where to quote, how deep, and when to cross.
//!
//! A deployed exchange nobody is trading on shows a frozen book, an empty tape
//! and a chart that stopped forming — which reads as broken even though every
//! part of it is working. This module decides what a small resident maker
//! should do about that.
//!
//! Everything here is pure arithmetic over a seeded generator, so a demo run is
//! reproducible from its seed and every decision is testable without a running
//! exchange. The daemon in `bin/demo-maker.rs` does the I/O and nothing else.

use cex_proto::Side;

/// One order the maker wants resting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    pub side: Side,
    pub price: i64,
    pub qty: i64,
}

/// A ladder of resting quotes, `levels` deep on each side of `mid`.
///
/// Size grows with distance from the touch. That is not decoration: the
/// ladder's depth histogram is cumulative, so a flat ladder draws a straight
/// diagonal and says nothing, where a book that thickens as it goes out draws
/// the curve a real one has.
///
/// Prices at or below zero are dropped rather than clamped — a market whose
/// mid has walked into the floor should quote less, not quote nonsense.
pub fn ladder(mid: i64, tick: i64, levels: usize, base_qty: i64) -> Vec<Quote> {
    let mut out = Vec::with_capacity(levels * 2);
    for i in 1..=levels as i64 {
        let qty = base_qty * i;
        let bid = mid - i * tick;
        if bid > 0 {
            out.push(Quote {
                side: Side::Buy,
                price: bid,
                qty,
            });
        }
        out.push(Quote {
            side: Side::Sell,
            price: mid + i * tick,
            qty,
        });
    }
    out
}

/// Move the mid by `steps` ticks, held inside `band`.
///
/// The band is what keeps a long run from wandering somewhere absurd. A demo
/// venue that drifts to 12,000 over a weekend is no more convincing than one
/// frozen at 50,000.
pub fn walk(mid: i64, tick: i64, steps: i64, band: (i64, i64)) -> i64 {
    (mid + steps * tick).clamp(band.0, band.1)
}

/// A small xorshift generator.
///
/// Deliberately not a dependency: this picks which way a demo price drifts and
/// how often it trades. Seeding it means a run can be replayed exactly, which
/// a cryptographic generator would not give and a demo does not need.
pub struct Rng(u64);

impl Rng {
    /// `seed` is forced odd, because xorshift is stuck at zero.
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform-ish in `0..n`. Returns 0 when `n` is 0 rather than dividing by it.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        self.next_u64() % n
    }

    /// One of -1, 0, +1 — a step for the mid to take.
    pub fn drift(&mut self) -> i64 {
        self.below(3) as i64 - 1
    }

    /// True about one time in `n`.
    pub fn one_in(&mut self, n: u64) -> bool {
        self.below(n) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: i64 = 10_000;
    const MID: i64 = 50_000_000_000;

    #[test]
    fn quotes_both_sides_to_the_requested_depth() {
        let book = ladder(MID, TICK, 12, 100);
        assert_eq!(book.len(), 24);
        assert_eq!(book.iter().filter(|q| q.side == Side::Buy).count(), 12);
        assert_eq!(book.iter().filter(|q| q.side == Side::Sell).count(), 12);
    }

    #[test]
    fn never_quotes_a_bid_at_or_above_an_ask() {
        // The maker quotes both sides of one book; a ladder that crossed itself
        // would trade with itself the moment it was placed.
        let book = ladder(MID, TICK, 12, 100);
        let best_bid = book
            .iter()
            .filter(|q| q.side == Side::Buy)
            .map(|q| q.price)
            .max()
            .unwrap();
        let best_ask = book
            .iter()
            .filter(|q| q.side == Side::Sell)
            .map(|q| q.price)
            .min()
            .unwrap();
        assert!(best_bid < best_ask, "{best_bid} !< {best_ask}");
        assert_eq!(best_ask - best_bid, 2 * TICK);
    }

    #[test]
    fn size_grows_with_distance_from_the_touch() {
        let book = ladder(MID, TICK, 5, 100);
        let mut asks: Vec<_> = book.iter().filter(|q| q.side == Side::Sell).collect();
        asks.sort_by_key(|q| q.price);
        let sizes: Vec<i64> = asks.iter().map(|q| q.qty).collect();
        assert_eq!(sizes, vec![100, 200, 300, 400, 500]);
    }

    #[test]
    fn drops_bids_that_would_price_at_or_below_zero() {
        // Five ticks below a two-tick mid is negative. Quoting it would be
        // rejected by the engine on every cycle, forever.
        let book = ladder(2 * TICK, TICK, 5, 100);
        assert!(book
            .iter()
            .filter(|q| q.side == Side::Buy)
            .all(|q| q.price > 0));
        assert_eq!(book.iter().filter(|q| q.side == Side::Buy).count(), 1);
        // The ask side is unaffected — it only ever moves away from zero.
        assert_eq!(book.iter().filter(|q| q.side == Side::Sell).count(), 5);
    }

    #[test]
    fn the_mid_walks_but_stays_in_its_band() {
        let band = (MID - 100 * TICK, MID + 100 * TICK);
        assert_eq!(walk(MID, TICK, 1, band), MID + TICK);
        assert_eq!(walk(MID, TICK, -1, band), MID - TICK);
        assert_eq!(walk(MID, TICK, 0, band), MID);
        // And it cannot leave, however long it is pushed one way.
        assert_eq!(walk(band.1, TICK, 5, band), band.1);
        assert_eq!(walk(band.0, TICK, -5, band), band.0);
    }

    #[test]
    fn the_generator_replays_from_its_seed() {
        let draw = |seed| {
            let mut rng = Rng::new(seed);
            (0..16).map(|_| rng.next_u64()).collect::<Vec<_>>()
        };
        assert_eq!(draw(7), draw(7));
        assert_ne!(draw(7), draw(8));
    }

    #[test]
    fn a_zero_seed_still_generates() {
        // Xorshift is stuck at zero, so the seed is forced odd on the way in.
        let mut rng = Rng::new(0);
        assert_ne!(rng.next_u64(), 0);
    }

    #[test]
    fn drift_covers_down_flat_and_up() {
        let mut rng = Rng::new(1);
        let mut seen = [false; 3];
        for _ in 0..500 {
            let d = rng.drift();
            assert!((-1..=1).contains(&d), "drift out of range: {d}");
            seen[(d + 1) as usize] = true;
        }
        assert_eq!(seen, [true, true, true]);
    }

    #[test]
    fn one_in_n_is_neither_never_nor_always() {
        let mut rng = Rng::new(42);
        let hits = (0..600).filter(|_| rng.one_in(6)).count();
        assert!(
            hits > 30 && hits < 170,
            "one_in(6) fired {hits} times in 600"
        );
    }

    #[test]
    fn below_zero_does_not_divide_by_zero() {
        let mut rng = Rng::new(3);
        assert_eq!(rng.below(0), 0);
    }
}
