//! Every message must survive a round trip through JSON.
//!
//! This file exists because it did not, and nothing caught it. `ResponseBody`
//! was internally tagged, which serde cannot do for a variant holding a
//! sequence — so any response carrying a list failed to serialise *at runtime*,
//! with the error swallowed by a log line. The command path happened to use only
//! unit and struct variants, so every test passed.
//!
//! The rule this encodes: if a type crosses a process boundary, prove it can.

use cex_proto::*;
use uuid::Uuid;

fn rid() -> Uuid {
    Uuid::from_u128(1)
}

fn uid() -> Uuid {
    Uuid::from_u128(2)
}

/// Encode, decode, and check we got back what we started with.
fn round_trip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + PartialEq,
{
    let json =
        serde_json::to_string(value).unwrap_or_else(|e| panic!("failed to ENCODE {value:?}: {e}"));
    let back: T =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("failed to DECODE {json}: {e}"));
    assert_eq!(
        &back, value,
        "round trip changed the value\njson was: {json}"
    );
    back
}

fn balance_view() -> BalanceView {
    BalanceView {
        asset: "USDT".into(),
        available: 100,
        locked: 25,
    }
}

fn order_view() -> OrderView {
    OrderView {
        order_id: 7,
        user_id: uid(),
        symbol: "BTC_USDT".into(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        price: Some(50_000_000_000),
        qty: 100_000,
        filled_qty: 0,
        status: OrderStatus::Open,
    }
}

fn market_view() -> MarketView {
    MarketView {
        symbol: "BTC_USDT".into(),
        base: "BTC".into(),
        quote: "USDT".into(),
        base_decimals: 8,
        quote_decimals: 6,
        tick_size: 10_000,
        lot_size: 1_000,
        min_notional: 1_000_000,
        maker_fee_bps: 2,
        taker_fee_bps: 5,
    }
}

// ───────────────────────── response bodies ─────────────────────────
// One test per variant, so a failure names the variant that broke.

#[test]
fn ack_round_trips() {
    round_trip(&ResponseBody::Ack);
}

#[test]
fn balances_round_trips() {
    round_trip(&ResponseBody::Balances(vec![
        balance_view(),
        balance_view(),
    ]));
}

#[test]
fn an_empty_list_body_round_trips() {
    round_trip(&ResponseBody::Balances(vec![]));
}

#[test]
fn depth_round_trips() {
    round_trip(&ResponseBody::Depth(DepthSnapshot {
        symbol: "BTC_USDT".into(),
        depth_seq: 42,
        bids: vec![[49_000_000_000, 100_000]],
        asks: vec![[50_000_000_000, 200_000]],
    }));
}

#[test]
fn order_round_trips() {
    round_trip(&ResponseBody::Order(order_view()));
}

#[test]
fn orders_round_trips() {
    round_trip(&ResponseBody::Orders(vec![order_view()]));
}

#[test]
fn markets_round_trips() {
    round_trip(&ResponseBody::Markets(vec![market_view()]));
}

#[test]
fn order_placed_round_trips() {
    round_trip(&ResponseBody::OrderPlaced {
        order_id: 3,
        status: OrderStatus::PartiallyFilled,
        filled_qty: 50_000,
        qty: 100_000,
        avg_price: Some(49_500_000_000),
    });
}

// ───────────────────────── the envelope ─────────────────────────

#[test]
fn a_successful_response_round_trips_for_every_body() {
    // The combination is what actually goes on the wire, and it is where the
    // original bug lived: a body that encodes alone can still fail once wrapped.
    let bodies = vec![
        ResponseBody::Ack,
        ResponseBody::Balances(vec![balance_view()]),
        ResponseBody::Orders(vec![order_view()]),
        ResponseBody::Markets(vec![market_view()]),
        ResponseBody::Order(order_view()),
    ];
    for body in bodies {
        round_trip(&Response::ok(rid(), body));
    }
}

#[test]
fn an_error_response_round_trips() {
    round_trip(&Response::err(rid(), "unknown market NOPE_USDT"));
}

#[test]
fn success_and_failure_are_distinguishable_after_a_round_trip() {
    // An error must never decode as a success or the caller acts on nothing.
    let ok = round_trip(&Response::ok(rid(), ResponseBody::Ack));
    let err = round_trip(&Response::err(rid(), "nope"));

    assert!(matches!(ok.result, ResponseResult::Ok { .. }));
    assert!(matches!(err.result, ResponseResult::Err { .. }));
}

#[test]
fn a_response_carries_the_request_id_it_answers() {
    let r = round_trip(&Response::ok(rid(), ResponseBody::Ack));
    assert_eq!(r.request_id, rid());
}

// ───────────────────────── commands ─────────────────────────

#[test]
fn every_command_round_trips() {
    let commands = vec![
        Command::Deposit {
            request_id: rid(),
            user_id: uid(),
            asset: "USDT".into(),
            amount: 1_000,
        },
        Command::Withdraw {
            request_id: rid(),
            user_id: uid(),
            asset: "BTC".into(),
            amount: 5,
        },
        Command::PlaceOrder {
            request_id: rid(),
            user_id: uid(),
            symbol: "BTC_USDT".into(),
            side: Side::Sell,
            order_type: OrderType::Market,
            time_in_force: None,
            price: None,
            qty: 100_000,
        },
        Command::CancelOrder {
            request_id: rid(),
            user_id: uid(),
            order_id: 9,
        },
    ];
    for c in commands {
        round_trip(&c);
    }
}

#[test]
fn a_place_order_command_omitting_optional_fields_still_decodes() {
    // The API may leave time_in_force and price out for a market order.
    let json = format!(
        r#"{{"cmd":"place_order","request_id":"{}","user_id":"{}",
             "symbol":"BTC_USDT","side":"BUY","order_type":"MARKET","qty":100000}}"#,
        rid(),
        uid()
    );
    let cmd: Command = serde_json::from_str(&json).expect("should decode");
    match cmd {
        Command::PlaceOrder {
            time_in_force,
            price,
            ..
        } => {
            assert!(time_in_force.is_none());
            assert!(price.is_none());
        }
        other => panic!("expected PlaceOrder, got {other:?}"),
    }
}

// ───────────────────────── queries ─────────────────────────

#[test]
fn every_query_round_trips() {
    let queries = vec![
        Query::Depth {
            request_id: rid(),
            symbol: "BTC_USDT".into(),
            limit: Some(20),
        },
        Query::Balances {
            request_id: rid(),
            user_id: uid(),
        },
        Query::Order {
            request_id: rid(),
            user_id: uid(),
            order_id: 4,
        },
        Query::OpenOrders {
            request_id: rid(),
            user_id: uid(),
            symbol: None,
        },
        Query::Markets { request_id: rid() },
    ];
    for q in queries {
        round_trip(&q);
    }
}

// ───────────────────────── events ─────────────────────────

#[test]
fn every_event_round_trips() {
    let events = vec![
        Event::Deposited {
            user_id: uid(),
            asset: "USDT".into(),
            amount: 100,
            available: 100,
        },
        Event::Withdrawn {
            user_id: uid(),
            asset: "USDT".into(),
            amount: 10,
            available: 90,
        },
        Event::OrderAccepted {
            order_id: 1,
            user_id: uid(),
            symbol: "BTC_USDT".into(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(50_000_000_000),
            qty: 100_000,
        },
        Event::OrderRejected {
            user_id: uid(),
            symbol: "BTC_USDT".into(),
            reason: "insufficient balance".into(),
        },
        Event::Trades {
            symbol: "BTC_USDT".into(),
            fills: vec![Fill {
                symbol: "BTC_USDT".into(),
                price: 49_000_000_000,
                qty: 100_000,
                maker_order_id: 1,
                taker_order_id: 2,
                maker_user_id: uid(),
                taker_user_id: rid(),
                taker_side: Side::Buy,
                notional: 49_000_000,
                maker_fee: 9_800,
                taker_fee: 50,
            }],
        },
        Event::OrderUpdated {
            order_id: 1,
            user_id: uid(),
            filled_qty: 100_000,
            qty: 100_000,
            status: OrderStatus::Filled,
        },
        Event::OrderCancelled {
            order_id: 1,
            user_id: uid(),
            symbol: "BTC_USDT".into(),
            unfilled_qty: 0,
        },
        Event::DepthUpdated {
            symbol: "BTC_USDT".into(),
            depth_seq: 8,
            deltas: vec![DepthDelta {
                side: Side::Buy,
                price: 49_000_000_000,
                qty: 0,
            }],
        },
        Event::BalanceUpdated {
            user_id: uid(),
            asset: "USDT".into(),
            available: 10,
            locked: 5,
        },
    ];
    for e in events {
        round_trip(&e);
    }
}

#[test]
fn an_event_batch_round_trips() {
    round_trip(&EventBatch {
        seq: 12,
        request_id: rid(),
        events: vec![
            Event::BalanceUpdated {
                user_id: uid(),
                asset: "USDT".into(),
                available: 1,
                locked: 0,
            },
            Event::Trades {
                symbol: "BTC_USDT".into(),
                fills: vec![],
            },
        ],
    });
}

#[test]
fn an_empty_event_batch_round_trips() {
    round_trip(&EventBatch {
        seq: 1,
        request_id: rid(),
        events: vec![],
    });
}
