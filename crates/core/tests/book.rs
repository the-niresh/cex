//! Behavioural tests for the order book.
//!
//! Several of these exist because a specific reference implementation got the
//! case wrong; those are named after the behaviour, and the comment says which
//! project failed it. They are the reason this file is written before the book is.

use cex_core::book::{Order, OrderBook};
use cex_core::market::MarketRegistry;
use cex_proto::{OrderStatus, OrderType, Side, TimeInForce};
use uuid::Uuid;

// BTC_USDT: tick 0.01 USDT, lot 0.00001 BTC, base 8dp, quote 6dp.
const P49K: i64 = 49_000_000_000; // 49,000.00 USDT per BTC
const P50K: i64 = 50_000_000_000; // 50,000.00
const P51K: i64 = 51_000_000_000; // 51,000.00
const Q1: i64 = 100_000; // 0.001 BTC
const Q2: i64 = 200_000; // 0.002 BTC

fn book() -> OrderBook {
    OrderBook::new("BTC_USDT")
}

fn user() -> Uuid {
    Uuid::new_v4()
}

fn limit(id: u64, who: Uuid, side: Side, price: i64, qty: i64) -> Order {
    Order::limit(id, who, side, price, qty, TimeInForce::Gtc)
}

fn ioc(id: u64, who: Uuid, side: Side, price: i64, qty: i64) -> Order {
    Order::limit(id, who, side, price, qty, TimeInForce::Ioc)
}

fn market(id: u64, who: Uuid, side: Side, qty: i64) -> Order {
    Order::market(id, who, side, qty)
}

// ───────────────────────── resting and depth ─────────────────────────

#[test]
fn a_limit_order_with_no_counterparty_rests_on_its_own_side() {
    let mut b = book();
    let out = b.place(limit(1, user(), Side::Buy, P50K, Q1));

    assert!(out.fills.is_empty());
    assert!(out.rested);
    assert_eq!(b.best_bid(), Some(P50K));
    assert_eq!(b.best_ask(), None);
    assert_eq!(b.depth(Side::Buy, 10), vec![[P50K, Q1]]);
    assert_eq!(b.order(1).unwrap().status, OrderStatus::Open);
}

#[test]
fn depth_is_returned_best_price_first_on_both_sides() {
    let mut b = book();
    b.place(limit(1, user(), Side::Buy, P49K, Q1));
    b.place(limit(2, user(), Side::Buy, P50K, Q1));
    b.place(limit(3, user(), Side::Sell, P51K, Q1));
    b.place(limit(4, user(), Side::Sell, P51K + 10_000, Q1));

    // Bids descend from the best (highest) bid.
    assert_eq!(b.depth(Side::Buy, 10), vec![[P50K, Q1], [P49K, Q1]]);
    // Asks ascend from the best (lowest) ask.
    assert_eq!(
        b.depth(Side::Sell, 10),
        vec![[P51K, Q1], [P51K + 10_000, Q1]]
    );
}

#[test]
fn orders_at_the_same_price_aggregate_into_one_level() {
    let mut b = book();
    b.place(limit(1, user(), Side::Buy, P50K, Q1));
    b.place(limit(2, user(), Side::Buy, P50K, Q2));

    assert_eq!(b.depth(Side::Buy, 10), vec![[P50K, Q1 + Q2]]);
}

// ───────────────────────── the crossing test ─────────────────────────
// `cex` inverted this: it broke out of the match loop in exactly the case
// where a trade should happen, so limit orders only ever filled at prices
// worse than the limit.

#[test]
fn a_buy_crosses_an_ask_below_its_limit() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P49K, Q1));
    let out = b.place(limit(2, user(), Side::Buy, P50K, Q1));

    assert_eq!(out.fills.len(), 1, "a buy at 50k must take an ask at 49k");
    assert_eq!(out.fills[0].qty, Q1);
}

#[test]
fn a_buy_crosses_an_ask_exactly_at_its_limit() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));
    let out = b.place(limit(2, user(), Side::Buy, P50K, Q1));

    assert_eq!(out.fills.len(), 1, "equal prices must cross");
}

#[test]
fn a_buy_does_not_cross_an_ask_above_its_limit() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P51K, Q1));
    let out = b.place(limit(2, user(), Side::Buy, P50K, Q1));

    assert!(
        out.fills.is_empty(),
        "a buy at 50k must not take an ask at 51k"
    );
    assert_eq!(b.best_bid(), Some(P50K));
    assert_eq!(b.best_ask(), Some(P51K));
}

#[test]
fn a_sell_crosses_a_bid_at_or_above_its_limit_but_not_below() {
    let mut b = book();
    b.place(limit(1, user(), Side::Buy, P50K, Q1));

    let no = b.place(limit(2, user(), Side::Sell, P51K, Q1));
    assert!(
        no.fills.is_empty(),
        "a sell at 51k must not hit a bid at 50k"
    );

    let yes = b.place(limit(3, user(), Side::Sell, P49K, Q1));
    assert_eq!(yes.fills.len(), 1, "a sell at 49k must hit a bid at 50k");
}

// ───────────────────────── fill price ─────────────────────────
// `perp` used min(taker.price, maker.price), which hands price improvement
// to the wrong side whenever the taker is the seller.

#[test]
fn a_fill_prints_at_the_makers_price_when_the_taker_buys() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P49K, Q1));
    let out = b.place(limit(2, user(), Side::Buy, P51K, Q1));

    assert_eq!(out.fills[0].price, P49K, "resting ask sets the price");
}

#[test]
fn a_fill_prints_at_the_makers_price_when_the_taker_sells() {
    let mut b = book();
    b.place(limit(1, user(), Side::Buy, P51K, Q1));
    let out = b.place(limit(2, user(), Side::Sell, P49K, Q1));

    // min(51k, 49k) would be 49k and would rob the seller of the improvement.
    assert_eq!(out.fills[0].price, P51K, "resting bid sets the price");
}

// ───────────────────────── time priority ─────────────────────────

#[test]
fn orders_at_one_price_fill_in_arrival_order() {
    let mut b = book();
    let first = user();
    let second = user();
    b.place(limit(1, first, Side::Sell, P50K, Q1));
    b.place(limit(2, second, Side::Sell, P50K, Q1));

    let out = b.place(limit(3, user(), Side::Buy, P50K, Q1));

    assert_eq!(out.fills.len(), 1);
    assert_eq!(
        out.fills[0].maker_order_id, 1,
        "the older order must fill first"
    );
    assert_eq!(out.fills[0].maker_user_id, first);
}

#[test]
fn a_taker_sweeps_price_levels_from_best_to_worst() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P51K, Q1));
    b.place(limit(2, user(), Side::Sell, P49K, Q1));
    b.place(limit(3, user(), Side::Sell, P50K, Q1));

    let out = b.place(limit(4, user(), Side::Buy, P51K, Q1 * 3));

    let prices: Vec<i64> = out.fills.iter().map(|f| f.price).collect();
    assert_eq!(
        prices,
        vec![P49K, P50K, P51K],
        "cheapest ask consumed first"
    );
}

// ───────────────────────── partial fills ─────────────────────────

#[test]
fn an_oversized_taker_fills_what_it_can_and_rests_the_rest() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));
    let out = b.place(limit(2, user(), Side::Buy, P50K, Q1 + Q2));

    assert_eq!(out.fills.len(), 1);
    assert_eq!(out.fills[0].qty, Q1);
    assert!(out.rested);

    let taker = b.order(2).unwrap();
    assert_eq!(taker.filled_qty, Q1);
    assert_eq!(taker.status, OrderStatus::PartiallyFilled);
    assert_eq!(b.depth(Side::Buy, 10), vec![[P50K, Q2]]);
}

#[test]
fn a_partly_consumed_maker_stays_on_the_book_with_the_remainder() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1 + Q2));
    b.place(limit(2, user(), Side::Buy, P50K, Q1));

    let maker = b.order(1).unwrap();
    assert_eq!(maker.filled_qty, Q1);
    assert_eq!(maker.status, OrderStatus::PartiallyFilled);
    assert_eq!(b.depth(Side::Sell, 10), vec![[P50K, Q2]]);
}

#[test]
fn a_fully_consumed_level_is_removed_from_the_book() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));
    b.place(limit(2, user(), Side::Buy, P50K, Q1));

    assert_eq!(b.best_ask(), None);
    assert!(b.depth(Side::Sell, 10).is_empty());
    assert_eq!(b.order(1).unwrap().status, OrderStatus::Filled);
    assert_eq!(b.order(2).unwrap().status, OrderStatus::Filled);
}

// ───────────────────────── market orders ─────────────────────────
// `cex` never bounded a market buy by the requested quantity — the parameter
// was accepted and then ignored, so a funded buy could take more than asked.

#[test]
fn a_market_buy_never_takes_more_than_the_requested_quantity() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1 * 10));

    let out = b.place(market(2, user(), Side::Buy, Q1));

    let taken: i64 = out.fills.iter().map(|f| f.qty).sum();
    assert_eq!(taken, Q1, "must stop at the requested quantity");
    assert_eq!(b.depth(Side::Sell, 10), vec![[P50K, Q1 * 9]]);
}

#[test]
fn a_market_order_ignores_price_and_sweeps_whatever_is_there() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P49K, Q1));
    b.place(limit(2, user(), Side::Sell, P51K, Q1));

    let out = b.place(market(3, user(), Side::Buy, Q1 * 2));

    assert_eq!(out.fills.len(), 2);
    assert_eq!(out.fills[1].price, P51K, "no limit bound on a market order");
}

#[test]
fn a_market_order_does_not_rest_when_the_book_runs_out() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));

    let out = b.place(market(2, user(), Side::Buy, Q1 * 5));

    assert!(!out.rested, "an unfilled market remainder must never rest");
    assert!(b.depth(Side::Buy, 10).is_empty());
    assert_eq!(b.order(2).unwrap().status, OrderStatus::PartiallyFilled);
}

#[test]
fn a_market_order_against_an_empty_book_is_cancelled_outright() {
    let mut b = book();
    let out = b.place(market(1, user(), Side::Buy, Q1));

    assert!(out.fills.is_empty());
    assert!(!out.rested);
    assert_eq!(b.order(1).unwrap().status, OrderStatus::Cancelled);
}

// ───────────────────────── time in force ─────────────────────────

#[test]
fn an_ioc_remainder_is_cancelled_rather_than_rested() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));

    let out = b.place(ioc(2, user(), Side::Buy, P50K, Q1 + Q2));

    assert_eq!(out.fills.len(), 1);
    assert!(!out.rested);
    assert!(b.depth(Side::Buy, 10).is_empty());
    assert_eq!(b.order(2).unwrap().status, OrderStatus::PartiallyFilled);
}

// ───────────────────────── cancel ─────────────────────────

#[test]
fn a_cancelled_order_leaves_the_depth_immediately() {
    let mut b = book();
    b.place(limit(1, user(), Side::Buy, P50K, Q1));
    b.place(limit(2, user(), Side::Buy, P50K, Q2));

    let out = b.cancel(1).unwrap();

    assert_eq!(out.unfilled_qty, Q1);
    assert_eq!(b.depth(Side::Buy, 10), vec![[P50K, Q2]]);
    assert_eq!(b.order(1).unwrap().status, OrderStatus::Cancelled);
}

#[test]
fn cancelling_the_last_order_at_a_price_removes_the_level() {
    let mut b = book();
    b.place(limit(1, user(), Side::Buy, P50K, Q1));
    b.cancel(1).unwrap();

    assert_eq!(b.best_bid(), None);
    assert!(b.depth(Side::Buy, 10).is_empty());
}

#[test]
fn a_cancelled_order_is_skipped_when_a_taker_arrives() {
    // Cancel leaves a tombstone in the level queue. Matching must step over it
    // and reach the live order behind it, not trade against a dead order.
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));
    b.place(limit(2, user(), Side::Sell, P50K, Q1));
    b.cancel(1).unwrap();

    let out = b.place(limit(3, user(), Side::Buy, P50K, Q1));

    assert_eq!(out.fills.len(), 1);
    assert_eq!(out.fills[0].maker_order_id, 2, "must skip the tombstone");
}

#[test]
fn a_partly_filled_order_can_still_be_cancelled_for_its_remainder() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1 + Q2));
    b.place(limit(2, user(), Side::Buy, P50K, Q1));

    let out = b.cancel(1).unwrap();

    assert_eq!(out.unfilled_qty, Q2, "only the unfilled part is released");
}

#[test]
fn cancelling_twice_is_an_error_not_a_second_refund() {
    let mut b = book();
    b.place(limit(1, user(), Side::Buy, P50K, Q1));
    assert!(b.cancel(1).is_ok());
    assert!(
        b.cancel(1).is_err(),
        "a closed order cannot be cancelled again"
    );
}

#[test]
fn cancelling_a_filled_order_is_an_error() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));
    b.place(limit(2, user(), Side::Buy, P50K, Q1));

    assert!(b.cancel(1).is_err());
}

#[test]
fn cancelling_an_unknown_order_is_an_error() {
    let mut b = book();
    assert!(b.cancel(999).is_err());
}

// ───────────────────────── self-trade prevention ─────────────────────────

#[test]
fn a_taker_does_not_trade_against_its_own_resting_order() {
    let mut b = book();
    let alice = user();
    b.place(limit(1, alice, Side::Sell, P50K, Q1));

    let out = b.place(limit(2, alice, Side::Buy, P50K, Q1));

    assert!(out.fills.is_empty(), "alice must not trade with alice");
    assert_eq!(out.stp_cancelled.len(), 1);
    assert_eq!(out.stp_cancelled[0].order_id, 1);
    assert_eq!(out.stp_cancelled[0].unfilled_qty, Q1);
    assert_eq!(b.order(1).unwrap().status, OrderStatus::Cancelled);
}

#[test]
fn self_trade_prevention_removes_the_maker_then_keeps_matching() {
    let mut b = book();
    let alice = user();
    let bob = user();
    b.place(limit(1, alice, Side::Sell, P50K, Q1)); // alice's own, will be pulled
    b.place(limit(2, bob, Side::Sell, P50K, Q1)); // bob's, behind it in the queue

    let out = b.place(limit(3, alice, Side::Buy, P50K, Q1));

    assert_eq!(out.stp_cancelled.len(), 1);
    assert_eq!(
        out.fills.len(),
        1,
        "matching continues past the pulled order"
    );
    assert_eq!(out.fills[0].maker_user_id, bob);
}

// ───────────────────────── simulation ─────────────────────────
// A market buy needs an exact reservation up front. This is the honest version
// of `cex`'s average-price estimate, which could not be exact by construction.

#[test]
fn simulate_reports_the_exact_cost_of_sweeping_two_levels() {
    let reg = MarketRegistry::with_defaults();
    let m = reg.get("BTC_USDT").unwrap();
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P49K, Q1));
    b.place(limit(2, user(), Side::Sell, P50K, Q1));

    let sim = b.simulate(m, Side::Buy, Q1 * 2, None).unwrap();

    // 0.001 BTC at 49,000 = 49 USDT, plus 0.001 at 50,000 = 50 USDT.
    assert_eq!(sim.fillable_qty, Q1 * 2);
    assert_eq!(sim.cost, 49_000_000 + 50_000_000);
}

#[test]
fn simulate_reports_short_fill_when_the_book_is_thin() {
    let reg = MarketRegistry::with_defaults();
    let m = reg.get("BTC_USDT").unwrap();
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));

    let sim = b.simulate(m, Side::Buy, Q1 * 5, None).unwrap();

    assert_eq!(
        sim.fillable_qty, Q1,
        "cannot promise more than the book holds"
    );
    assert_eq!(sim.cost, 50_000_000);
}

#[test]
fn simulate_excludes_the_takers_own_orders() {
    let reg = MarketRegistry::with_defaults();
    let m = reg.get("BTC_USDT").unwrap();
    let alice = user();
    let mut b = book();
    b.place(limit(1, alice, Side::Sell, P49K, Q1));
    b.place(limit(2, user(), Side::Sell, P50K, Q1));

    let sim = b.simulate(m, Side::Buy, Q1 * 2, Some(alice)).unwrap();

    // Alice's own 49k ask will be pulled by STP, so it must not be quoted to her.
    assert_eq!(sim.fillable_qty, Q1);
    assert_eq!(sim.cost, 50_000_000);
}

#[test]
fn simulate_does_not_mutate_the_book() {
    let reg = MarketRegistry::with_defaults();
    let m = reg.get("BTC_USDT").unwrap();
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));

    let before = b.depth(Side::Sell, 10);
    b.simulate(m, Side::Buy, Q1, None).unwrap();
    assert_eq!(b.depth(Side::Sell, 10), before);
}

// ───────────────────────── depth feed ─────────────────────────

#[test]
fn the_depth_sequence_advances_once_per_mutating_command() {
    let mut b = book();
    let start = b.depth_seq();

    b.place(limit(1, user(), Side::Buy, P50K, Q1));
    let after_place = b.depth_seq();
    assert!(after_place > start, "resting an order changes the book");

    b.cancel(1).unwrap();
    assert!(b.depth_seq() > after_place, "cancelling changes the book");
}

#[test]
fn the_depth_sequence_does_not_advance_when_nothing_changed() {
    let mut b = book();
    let seq = b.depth_seq();

    // A market buy against an empty book touches no level and rests nothing.
    b.place(market(1, user(), Side::Buy, Q1));

    assert_eq!(b.depth_seq(), seq, "a no-op must not bump the sequence");
}

#[test]
fn an_emptied_level_is_reported_as_zero_quantity_not_omitted() {
    // A diff consumer needs an explicit zero to know to delete the level.
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));
    let out = b.place(limit(2, user(), Side::Buy, P50K, Q1));

    let deltas = b.deltas_for(&out.touched);
    let ask_delta = deltas
        .iter()
        .find(|d| d.side == Side::Sell && d.price == P50K)
        .expect("the emptied ask level must appear in the diff");
    assert_eq!(ask_delta.qty, 0);
}

#[test]
fn a_resting_order_is_reported_with_its_new_level_total() {
    let mut b = book();
    b.place(limit(1, user(), Side::Buy, P50K, Q1));
    let out = b.place(limit(2, user(), Side::Buy, P50K, Q2));

    let deltas = b.deltas_for(&out.touched);
    let bid = deltas
        .iter()
        .find(|d| d.side == Side::Buy && d.price == P50K)
        .expect("the bid level must appear in the diff");
    assert_eq!(
        bid.qty,
        Q1 + Q2,
        "diffs carry the level total, not the delta"
    );
}

// ───────────────────────── last traded price ─────────────────────────

#[test]
fn the_last_traded_price_tracks_the_most_recent_fill() {
    let mut b = book();
    assert_eq!(b.last_price(), None);

    b.place(limit(1, user(), Side::Sell, P49K, Q1));
    b.place(limit(2, user(), Side::Sell, P50K, Q1));
    b.place(limit(3, user(), Side::Buy, P50K, Q1 * 2));

    assert_eq!(
        b.last_price(),
        Some(P50K),
        "the last fill of the sweep wins"
    );
}

#[test]
fn an_order_that_does_not_trade_leaves_the_last_price_alone() {
    let mut b = book();
    b.place(limit(1, user(), Side::Sell, P50K, Q1));
    b.place(limit(2, user(), Side::Buy, P49K, Q1));

    assert_eq!(b.last_price(), None);
}

// ───────────────────────── order type helpers ─────────────────────────

#[test]
fn a_limit_order_is_constructed_with_the_fields_it_was_given() {
    let who = user();
    let o = limit(7, who, Side::Buy, P50K, Q1);

    assert_eq!(o.id, 7);
    assert_eq!(o.user_id, who);
    assert_eq!(o.side, Side::Buy);
    assert_eq!(o.order_type, OrderType::Limit);
    assert_eq!(o.price, P50K);
    assert_eq!(o.qty, Q1);
    assert_eq!(o.filled_qty, 0);
    assert_eq!(o.remaining(), Q1);
    assert_eq!(o.status, OrderStatus::Open);
}

#[test]
fn a_market_order_carries_no_price() {
    let o = market(8, user(), Side::Sell, Q1);
    assert_eq!(o.order_type, OrderType::Market);
    assert_eq!(o.price, 0);
    // A market order is inherently immediate-or-cancel.
    assert_eq!(o.time_in_force, TimeInForce::Ioc);
}
