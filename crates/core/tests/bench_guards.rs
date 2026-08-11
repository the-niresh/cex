//! The benchmark in `benches/apply.rs` is only measuring matching if these two
//! things hold. Both fail silently — the bench still produces a number, just
//! not the one it claims. These tests are the alarm.

use cex_core::{MarketRegistry, State};
use cex_proto::{Command, OrderType, Side, TimeInForce};
use uuid::Uuid;

const SYMBOL: &str = "BTC_USDT";
const PRICE: i64 = 50_000_000_000;
const QTY: i64 = 100_000;

fn funded_state() -> (State, Uuid) {
    let mut state = State::new(MarketRegistry::with_defaults());
    let user = Uuid::new_v4();
    for asset in ["USDT", "BTC"] {
        state
            .apply(Command::Deposit {
                request_id: Uuid::new_v4(),
                user_id: user,
                asset: asset.to_string(),
                amount: 1_000_000_000_000_000,
            })
            .expect("deposit");
    }
    (state, user)
}

fn place(user: Uuid, price: i64) -> Command {
    Command::PlaceOrder {
        request_id: Uuid::new_v4(),
        user_id: user,
        symbol: SYMBOL.to_string(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        time_in_force: Some(TimeInForce::Gtc),
        price: Some(price),
        qty: QTY,
    }
}

/// Count of resting bid orders in the book. `OrderBook` has no `bids()`
/// accessor; `orders()` filtered to live buy-side orders is the public API
/// that gives the same signal — the book grew by one resting order.
fn resting_bid_count(book: &cex_core::OrderBook) -> usize {
    book.orders()
        .filter(|o| o.side == Side::Buy && o.is_live())
        .count()
}

#[test]
fn a_repeated_request_id_produces_no_events() {
    let (mut state, user) = funded_state();
    let cmd = place(user, PRICE);

    let first = state.apply(cmd.clone()).expect("first apply");
    let second = state.apply(cmd).expect("second apply");

    assert!(
        !first.events.is_empty(),
        "the first apply must do real work"
    );
    assert!(
        second.events.is_empty(),
        "a repeated request_id short-circuits on the idempotency log, so a \
         benchmark that reuses one is timing a hashmap lookup"
    );
}

#[test]
fn cloning_state_isolates_the_clone_from_the_original() {
    let (state, user) = funded_state();
    let before = resting_bid_count(state.book(SYMBOL).expect("book"));

    let mut clone = state.clone();
    clone.apply(place(user, PRICE)).expect("apply to clone");

    let after_original = resting_bid_count(state.book(SYMBOL).expect("book"));
    let after_clone = resting_bid_count(clone.book(SYMBOL).expect("book"));

    assert_eq!(
        before, after_original,
        "applying to a clone must not touch the original, or iter_batched \
         would benchmark a book that grows across iterations"
    );
    assert_eq!(after_clone, before + 1);
}
