//! Behavioural tests for `State::apply` — the engine's whole public surface.
//!
//! This is where the book and the ledger meet, so this is where money is created
//! or destroyed if anything is wrong. Every trade test ends by asserting the
//! conservation invariant.

use cex_core::state::{State, FEE_ACCOUNT};
use cex_core::MarketRegistry;
use cex_proto::{Command, OrderStatus, OrderType, Query, ResponseBody, Side, TimeInForce};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const USDT: &str = "USDT";
const BTC: &str = "BTC";

// BTC_USDT: tick 0.01 USDT, lot 0.00001 BTC, base 8dp, quote 6dp,
// maker 2 bps, taker 5 bps, min notional 1 USDT.
const P49K: i64 = 49_000_000_000;
const P50K: i64 = 50_000_000_000;
const P51K: i64 = 51_000_000_000;
const Q1: i64 = 100_000; // 0.001 BTC
const N1: i64 = 50_000_000; // notional of Q1 at P50K = 50 USDT

const FUND_USDT: i64 = 1_000_000_000; // 1,000 USDT
const FUND_BTC: i64 = 100_000_000; // 1 BTC

fn state() -> State {
    State::new(MarketRegistry::with_defaults())
}

fn rid() -> Uuid {
    Uuid::new_v4()
}

fn deposit(s: &mut State, who: Uuid, asset: &str, amount: i64) {
    s.apply(Command::Deposit {
        request_id: rid(),
        user_id: who,
        asset: asset.to_string(),
        amount,
    })
    .expect("deposit should succeed");
}

fn funded(s: &mut State) -> (Uuid, Uuid) {
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    deposit(s, alice, USDT, FUND_USDT);
    deposit(s, bob, BTC, FUND_BTC);
    (alice, bob)
}

fn limit(who: Uuid, side: Side, price: i64, qty: i64) -> Command {
    Command::PlaceOrder {
        request_id: rid(),
        user_id: who,
        symbol: SYM.to_string(),
        side,
        order_type: OrderType::Limit,
        time_in_force: Some(TimeInForce::Gtc),
        price: Some(price),
        qty,
    }
}

fn market_order(who: Uuid, side: Side, qty: i64) -> Command {
    Command::PlaceOrder {
        request_id: rid(),
        user_id: who,
        symbol: SYM.to_string(),
        side,
        order_type: OrderType::Market,
        time_in_force: None,
        price: None,
        qty,
    }
}

fn order_id_of(body: &ResponseBody) -> u64 {
    match body {
        ResponseBody::OrderPlaced { order_id, .. } => *order_id,
        other => panic!("expected OrderPlaced, got {other:?}"),
    }
}

// ───────────────────────── deposits and withdrawals ─────────────────────────

#[test]
fn a_deposit_credits_available_and_raises_recorded_supply() {
    let mut s = state();
    let who = Uuid::new_v4();

    deposit(&mut s, who, USDT, FUND_USDT);

    assert_eq!(s.balances().get(who, USDT).available, FUND_USDT);
    s.check_invariants().expect("invariants after deposit");
}

#[test]
fn a_deposit_of_an_unlisted_asset_is_rejected() {
    let mut s = state();
    let got = s.apply(Command::Deposit {
        request_id: rid(),
        user_id: Uuid::new_v4(),
        asset: "DOGE".to_string(),
        amount: 100,
    });
    assert!(got.is_err());
}

#[test]
fn a_non_positive_deposit_is_rejected() {
    let mut s = state();
    let who = Uuid::new_v4();
    assert!(s
        .apply(Command::Deposit {
            request_id: rid(),
            user_id: who,
            asset: USDT.to_string(),
            amount: 0,
        })
        .is_err());
    assert!(s
        .apply(Command::Deposit {
            request_id: rid(),
            user_id: who,
            asset: USDT.to_string(),
            amount: -5,
        })
        .is_err());
}

#[test]
fn a_withdrawal_beyond_available_is_rejected_and_changes_nothing() {
    let mut s = state();
    let who = Uuid::new_v4();
    deposit(&mut s, who, USDT, 100);

    assert!(s
        .apply(Command::Withdraw {
            request_id: rid(),
            user_id: who,
            asset: USDT.to_string(),
            amount: 101,
        })
        .is_err());

    assert_eq!(s.balances().get(who, USDT).available, 100);
    s.check_invariants().unwrap();
}

#[test]
fn locked_funds_cannot_be_withdrawn() {
    let mut s = state();
    let (alice, _) = funded(&mut s);
    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();

    // The full balance minus the locked notional is still withdrawable.
    assert!(s
        .apply(Command::Withdraw {
            request_id: rid(),
            user_id: alice,
            asset: USDT.to_string(),
            amount: FUND_USDT - N1 + 1,
        })
        .is_err());
    assert!(s
        .apply(Command::Withdraw {
            request_id: rid(),
            user_id: alice,
            asset: USDT.to_string(),
            amount: FUND_USDT - N1,
        })
        .is_ok());
}

// ───────────────────────── reservation on placement ─────────────────────────

#[test]
fn a_resting_limit_buy_locks_exactly_the_notional() {
    let mut s = state();
    let (alice, _) = funded(&mut s);

    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();

    let bal = s.balances().get(alice, USDT);
    assert_eq!(bal.locked, N1, "locks price x qty");
    assert_eq!(bal.available, FUND_USDT - N1);
    s.check_invariants().unwrap();
}

#[test]
fn a_resting_limit_sell_locks_the_base_quantity() {
    let mut s = state();
    let (_, bob) = funded(&mut s);

    s.apply(limit(bob, Side::Sell, P50K, Q1)).unwrap();

    let bal = s.balances().get(bob, BTC);
    assert_eq!(bal.locked, Q1);
    assert_eq!(bal.available, FUND_BTC - Q1);
    s.check_invariants().unwrap();
}

#[test]
fn an_order_the_user_cannot_fund_is_rejected_and_locks_nothing() {
    let mut s = state();
    let poor = Uuid::new_v4();
    deposit(&mut s, poor, USDT, 1_000_000); // 1 USDT, needs 50

    let got = s.apply(limit(poor, Side::Buy, P50K, Q1));

    assert!(got.is_err());
    assert_eq!(s.balances().get(poor, USDT).locked, 0);
    assert_eq!(s.balances().get(poor, USDT).available, 1_000_000);
    s.check_invariants().unwrap();
}

// ───────────────────────── settlement ─────────────────────────

#[test]
fn a_fill_moves_base_one_way_and_quote_the_other() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);

    s.apply(limit(bob, Side::Sell, P50K, Q1)).unwrap(); // maker
    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap(); // taker

    // Alice paid 50 USDT and received BTC less the taker fee.
    let taker_fee_base = Q1 * 5 / 10_000; // 5 bps of 0.001 BTC
    assert_eq!(s.balances().get(alice, USDT).available, FUND_USDT - N1);
    assert_eq!(s.balances().get(alice, USDT).locked, 0);
    assert_eq!(s.balances().get(alice, BTC).available, Q1 - taker_fee_base);

    // Bob delivered BTC and received USDT less the maker fee.
    let maker_fee_quote = N1 * 2 / 10_000; // 2 bps of 50 USDT
    assert_eq!(s.balances().get(bob, BTC).available, FUND_BTC - Q1);
    assert_eq!(s.balances().get(bob, BTC).locked, 0);
    assert_eq!(s.balances().get(bob, USDT).available, N1 - maker_fee_quote);

    s.check_invariants().unwrap();
}

#[test]
fn the_taker_pays_the_taker_rate_and_the_maker_pays_the_maker_rate() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);

    s.apply(limit(bob, Side::Sell, P50K, Q1)).unwrap();
    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();

    // Alice took liquidity: 5 bps, charged in the asset she received (BTC).
    assert_eq!(
        s.balances().get(FEE_ACCOUNT, BTC).available,
        Q1 * 5 / 10_000
    );
    // Bob provided it: 2 bps, charged in the asset he received (USDT).
    assert_eq!(
        s.balances().get(FEE_ACCOUNT, USDT).available,
        N1 * 2 / 10_000
    );
}

#[test]
fn fees_accumulate_in_the_fee_account_rather_than_vanishing() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);

    s.apply(limit(bob, Side::Sell, P50K, Q1)).unwrap();
    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();

    let fees_btc = s.balances().get(FEE_ACCOUNT, BTC).available;
    let fees_usdt = s.balances().get(FEE_ACCOUNT, USDT).available;
    assert!(fees_btc > 0 && fees_usdt > 0);

    // Conservation only holds because the fees are still inside the ledger.
    s.check_invariants().unwrap();
}

// ───────────────────────── price improvement ─────────────────────────
// `cex` deducted quantity x limitPrice up front and credited each counterparty
// only the fill price, so the difference left the system entirely.

#[test]
fn a_buy_that_fills_below_its_limit_gets_the_difference_back() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);

    // Bob rests an ask at 49k. Alice buys with a limit of 51k.
    s.apply(limit(bob, Side::Sell, P49K, Q1)).unwrap();
    s.apply(limit(alice, Side::Buy, P51K, Q1)).unwrap();

    let paid = 49_000_000; // she trades at Bob's 49k, not her own 51k
    assert_eq!(
        s.balances().get(alice, USDT).available,
        FUND_USDT - paid,
        "the 2 USDT of price improvement must return to available"
    );
    assert_eq!(s.balances().get(alice, USDT).locked, 0);
    s.check_invariants().unwrap();
}

#[test]
fn a_sell_that_fills_above_its_limit_keeps_the_improvement() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);

    // Alice rests a bid at 51k. Bob sells with a limit of 49k.
    s.apply(limit(alice, Side::Buy, P51K, Q1)).unwrap();
    s.apply(limit(bob, Side::Sell, P49K, Q1)).unwrap();

    let received = 51_000_000; // he trades at Alice's 51k
    let taker_fee = received * 5 / 10_000;
    assert_eq!(
        s.balances().get(bob, USDT).available,
        received - taker_fee,
        "the seller keeps the improvement"
    );
    s.check_invariants().unwrap();
}

// ───────────────────────── cancel ─────────────────────────

#[test]
fn cancelling_returns_exactly_what_was_locked() {
    let mut s = state();
    let (alice, _) = funded(&mut s);

    let placed = s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();
    let id = order_id_of(&placed.response);
    assert_eq!(s.balances().get(alice, USDT).locked, N1);

    s.apply(Command::CancelOrder {
        request_id: rid(),
        user_id: alice,
        order_id: id,
    })
    .unwrap();

    assert_eq!(s.balances().get(alice, USDT).locked, 0);
    assert_eq!(s.balances().get(alice, USDT).available, FUND_USDT);
    s.check_invariants().unwrap();
}

#[test]
fn cancelling_a_partly_filled_order_returns_only_the_remainder() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);

    let placed = s.apply(limit(alice, Side::Buy, P50K, Q1 * 2)).unwrap();
    let id = order_id_of(&placed.response);
    s.apply(limit(bob, Side::Sell, P50K, Q1)).unwrap(); // fills half

    s.apply(Command::CancelOrder {
        request_id: rid(),
        user_id: alice,
        order_id: id,
    })
    .unwrap();

    assert_eq!(s.balances().get(alice, USDT).locked, 0);
    assert_eq!(
        s.balances().get(alice, USDT).available,
        FUND_USDT - N1,
        "she keeps what she spent, gets back what she did not"
    );
    s.check_invariants().unwrap();
}

#[test]
fn a_user_cannot_cancel_another_users_order() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);
    let placed = s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();
    let id = order_id_of(&placed.response);

    assert!(s
        .apply(Command::CancelOrder {
            request_id: rid(),
            user_id: bob,
            order_id: id,
        })
        .is_err());

    assert_eq!(s.balances().get(alice, USDT).locked, N1, "still locked");
}

// ───────────────────────── market orders ─────────────────────────

#[test]
fn a_market_buy_reserves_exactly_what_the_sweep_costs() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);

    s.apply(limit(bob, Side::Sell, P49K, Q1)).unwrap();
    s.apply(limit(bob, Side::Sell, P50K, Q1)).unwrap();

    s.apply(market_order(alice, Side::Buy, Q1 * 2)).unwrap();

    let spent = 49_000_000 + 50_000_000;
    assert_eq!(s.balances().get(alice, USDT).available, FUND_USDT - spent);
    assert_eq!(s.balances().get(alice, USDT).locked, 0, "nothing left over");
    s.check_invariants().unwrap();
}

#[test]
fn a_market_buy_against_a_thin_book_fills_what_it_can_and_releases_the_rest() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);
    s.apply(limit(bob, Side::Sell, P50K, Q1)).unwrap();

    s.apply(market_order(alice, Side::Buy, Q1 * 5)).unwrap();

    assert_eq!(s.balances().get(alice, USDT).available, FUND_USDT - N1);
    assert_eq!(s.balances().get(alice, USDT).locked, 0);
    s.check_invariants().unwrap();
}

#[test]
fn a_market_order_against_an_empty_book_is_rejected_without_locking() {
    let mut s = state();
    let (alice, _) = funded(&mut s);

    let got = s.apply(market_order(alice, Side::Buy, Q1));

    assert!(got.is_err(), "no liquidity means nothing to fill");
    assert_eq!(s.balances().get(alice, USDT).locked, 0);
    assert_eq!(s.balances().get(alice, USDT).available, FUND_USDT);
}

// ───────────────────────── self-trade ─────────────────────────

#[test]
fn a_pulled_self_trade_order_has_its_reservation_refunded() {
    let mut s = state();
    let (alice, _) = funded(&mut s);
    deposit(&mut s, alice, BTC, FUND_BTC);

    s.apply(limit(alice, Side::Sell, P50K, Q1)).unwrap();
    assert_eq!(s.balances().get(alice, BTC).locked, Q1);

    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();

    assert_eq!(
        s.balances().get(alice, BTC).locked,
        0,
        "the pulled ask must release its base"
    );
    assert_eq!(s.balances().get(alice, BTC).available, FUND_BTC);
    s.check_invariants().unwrap();
}

// ───────────────────────── validation ─────────────────────────

#[test]
fn a_misaligned_price_or_quantity_is_rejected() {
    let mut s = state();
    let (alice, _) = funded(&mut s);

    assert!(
        s.apply(limit(alice, Side::Buy, P50K + 1, Q1)).is_err(),
        "tick"
    );
    assert!(
        s.apply(limit(alice, Side::Buy, P50K, Q1 + 1)).is_err(),
        "lot"
    );
    s.check_invariants().unwrap();
}

#[test]
fn an_order_below_the_minimum_notional_is_rejected() {
    let mut s = state();
    let (alice, _) = funded(&mut s);

    // 1 lot at 50k = 0.5 USDT, under the 1 USDT floor.
    assert!(s.apply(limit(alice, Side::Buy, P50K, 1_000)).is_err());
}

#[test]
fn an_order_on_an_unlisted_market_is_rejected() {
    let mut s = state();
    let (alice, _) = funded(&mut s);

    let got = s.apply(Command::PlaceOrder {
        request_id: rid(),
        user_id: alice,
        symbol: "DOGE_USDT".to_string(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        time_in_force: Some(TimeInForce::Gtc),
        price: Some(P50K),
        qty: Q1,
    });
    assert!(got.is_err());
}

#[test]
fn a_limit_order_without_a_price_is_rejected() {
    let mut s = state();
    let (alice, _) = funded(&mut s);

    let got = s.apply(Command::PlaceOrder {
        request_id: rid(),
        user_id: alice,
        symbol: SYM.to_string(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        time_in_force: Some(TimeInForce::Gtc),
        price: None,
        qty: Q1,
    });
    assert!(got.is_err());
}

// ───────────────────────── sequencing ─────────────────────────

#[test]
fn the_sequence_advances_once_per_accepted_command() {
    let mut s = state();
    let (alice, _) = funded(&mut s);
    let before = s.seq();

    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();

    assert_eq!(s.seq(), before + 1);
}

#[test]
fn a_rejected_command_does_not_advance_the_sequence() {
    let mut s = state();
    let (alice, _) = funded(&mut s);
    let before = s.seq();

    let _ = s.apply(limit(alice, Side::Buy, P50K + 1, Q1)); // bad tick

    assert_eq!(s.seq(), before, "a rejection is not a state transition");
}

// ───────────────────────── queries ─────────────────────────

#[test]
fn a_depth_query_reflects_the_resting_book() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);
    s.apply(limit(alice, Side::Buy, P49K, Q1)).unwrap();
    s.apply(limit(bob, Side::Sell, P51K, Q1)).unwrap();

    let body = s
        .query(&Query::Depth {
            request_id: rid(),
            symbol: SYM.to_string(),
            limit: None,
        })
        .unwrap();

    match body {
        ResponseBody::Depth(d) => {
            assert_eq!(d.bids, vec![[P49K, Q1]]);
            assert_eq!(d.asks, vec![[P51K, Q1]]);
        }
        other => panic!("expected Depth, got {other:?}"),
    }
}

#[test]
fn a_balance_query_reports_available_and_locked_separately() {
    let mut s = state();
    let (alice, _) = funded(&mut s);
    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();

    let body = s
        .query(&Query::Balances {
            request_id: rid(),
            user_id: alice,
        })
        .unwrap();

    match body {
        ResponseBody::Balances(v) => {
            let usdt = v.iter().find(|b| b.asset == USDT).unwrap();
            assert_eq!(usdt.locked, N1, "locked must not be reported as zero");
            assert_eq!(usdt.available, FUND_USDT - N1);
        }
        other => panic!("expected Balances, got {other:?}"),
    }
}

#[test]
fn an_open_orders_query_lists_only_live_orders_for_that_user() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);
    let placed = s.apply(limit(alice, Side::Buy, P49K, Q1)).unwrap();
    let id = order_id_of(&placed.response);
    s.apply(limit(bob, Side::Sell, P51K, Q1)).unwrap();

    let body = s
        .query(&Query::OpenOrders {
            request_id: rid(),
            user_id: alice,
            symbol: None,
        })
        .unwrap();
    match &body {
        ResponseBody::Orders(v) => assert_eq!(v.len(), 1, "only alice's order"),
        other => panic!("expected Orders, got {other:?}"),
    }

    s.apply(Command::CancelOrder {
        request_id: rid(),
        user_id: alice,
        order_id: id,
    })
    .unwrap();

    let body = s
        .query(&Query::OpenOrders {
            request_id: rid(),
            user_id: alice,
            symbol: None,
        })
        .unwrap();
    match body {
        ResponseBody::Orders(v) => assert!(v.is_empty(), "cancelled orders are not open"),
        other => panic!("expected Orders, got {other:?}"),
    }
}

// ───────────────────────── conservation under load ─────────────────────────

#[test]
fn conservation_holds_across_a_long_mixed_command_sequence() {
    // The real test of the settlement code: run a lot of interleaved traffic and
    // assert after every single command that no atom was created or destroyed.
    let mut s = state();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    deposit(&mut s, alice, USDT, FUND_USDT * 100);
    deposit(&mut s, bob, USDT, FUND_USDT * 100);
    deposit(&mut s, alice, BTC, FUND_BTC * 10);
    deposit(&mut s, bob, BTC, FUND_BTC * 10);

    let mut placed_ids: Vec<u64> = Vec::new();
    let mut trade_count = 0usize;
    let mut seed: u64 = 0x5eed;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as i64
    };

    for i in 0..400 {
        let who = if i % 2 == 0 { alice } else { bob };
        let r = next();
        let cmd = match r % 4 {
            0 => {
                // Bids span 49,000-51,000 so they cross the ask band.
                let price = P49K + (r % 21) * 100_000_000;
                limit(who, Side::Buy, price - price % 10_000, Q1 * (1 + r % 3))
            }
            1 => {
                // Asks span the same range, guaranteeing crossing limit orders
                // and therefore price improvement to refund.
                let price = P49K + (r % 21) * 100_000_000;
                limit(who, Side::Sell, price - price % 10_000, Q1 * (1 + r % 3))
            }
            2 => market_order(who, if r % 2 == 0 { Side::Buy } else { Side::Sell }, Q1),
            _ => {
                if let Some(id) = placed_ids.pop() {
                    Command::CancelOrder {
                        request_id: rid(),
                        user_id: who,
                        order_id: id,
                    }
                } else {
                    limit(who, Side::Buy, P49K, Q1)
                }
            }
        };

        if let Ok(applied) = s.apply(cmd) {
            for e in &applied.events {
                if let cex_proto::Event::Trades { fills, .. } = e {
                    trade_count += fills.len();
                }
            }
            if let ResponseBody::OrderPlaced {
                order_id, status, ..
            } = applied.response
            {
                if !status.is_terminal() {
                    placed_ids.push(order_id);
                }
            }
        }

        s.check_invariants()
            .unwrap_or_else(|e| panic!("conservation broke at command {i}: {e}"));
    }

    // A conservation test that never trades proves nothing. Overlapping price
    // bands above guarantee crossing limit orders, not just market sweeps.
    assert!(
        trade_count > 50,
        "expected the random walk to actually trade, got {trade_count} fills"
    );
}

#[test]
fn every_locked_atom_is_backed_by_a_live_order() {
    // The other half of the invariant: locked funds must correspond to something
    // that can still release them. A leak here is money a user can never recover.
    let mut s = state();
    let (alice, bob) = funded(&mut s);

    s.apply(limit(alice, Side::Buy, P49K, Q1)).unwrap();
    s.apply(limit(bob, Side::Sell, P51K, Q1)).unwrap();
    s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();
    s.apply(market_order(bob, Side::Sell, Q1)).unwrap();

    s.check_invariants().unwrap();

    // Drain the book; every reservation must come back.
    let open: Vec<u64> = s.open_order_ids();
    for id in open {
        let owner = s.order_owner(id).unwrap();
        s.apply(Command::CancelOrder {
            request_id: rid(),
            user_id: owner,
            order_id: id,
        })
        .unwrap();
    }

    for asset in [USDT, BTC] {
        for who in [alice, bob] {
            assert_eq!(
                s.balances().get(who, asset).locked,
                0,
                "{asset} still locked for a user with no open orders"
            );
        }
    }
    s.check_invariants().unwrap();
}

#[test]
fn a_full_round_trip_returns_every_atom_to_its_owner() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);
    let placed = s.apply(limit(alice, Side::Buy, P50K, Q1)).unwrap();
    let id = order_id_of(&placed.response);

    s.apply(Command::CancelOrder {
        request_id: rid(),
        user_id: alice,
        order_id: id,
    })
    .unwrap();

    assert_eq!(s.balances().get(alice, USDT).available, FUND_USDT);
    assert_eq!(s.balances().get(bob, BTC).available, FUND_BTC);
    assert_eq!(
        s.balances().get(FEE_ACCOUNT, USDT).total(),
        0,
        "no fee on a cancel"
    );
}

// ───────────────────────── events ─────────────────────────

#[test]
fn a_trade_emits_a_trades_event_carrying_the_maker_price() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);
    s.apply(limit(bob, Side::Sell, P49K, Q1)).unwrap();

    let applied = s.apply(limit(alice, Side::Buy, P51K, Q1)).unwrap();

    let fills: Vec<_> = applied
        .events
        .iter()
        .filter_map(|e| match e {
            cex_proto::Event::Trades { fills, .. } => Some(fills.clone()),
            _ => None,
        })
        .flatten()
        .collect();

    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].price, P49K);
    assert_eq!(fills[0].taker_side, Side::Buy);
    assert_eq!(fills[0].maker_user_id, bob);
    assert_eq!(fills[0].taker_user_id, alice);
}

#[test]
fn placing_an_order_reports_its_status_and_average_fill_price() {
    let mut s = state();
    let (alice, bob) = funded(&mut s);
    s.apply(limit(bob, Side::Sell, P49K, Q1)).unwrap();
    s.apply(limit(bob, Side::Sell, P50K, Q1)).unwrap();

    let applied = s.apply(market_order(alice, Side::Buy, Q1 * 2)).unwrap();

    match applied.response {
        ResponseBody::OrderPlaced {
            status,
            filled_qty,
            avg_price,
            ..
        } => {
            assert_eq!(status, OrderStatus::Filled);
            assert_eq!(filled_qty, Q1 * 2);
            assert_eq!(avg_price, Some((P49K + P50K) / 2));
        }
        other => panic!("expected OrderPlaced, got {other:?}"),
    }
}
