//! Fanning one `EventBatch` out into addressed updates.
//!
//! No infrastructure: routing is a pure function, and the rule it exists to
//! keep — a public channel never carries a user id — is worth deciding in a
//! place a test can reach directly.

use cex_proto::{DepthDelta, Event, EventBatch, Fill, OrderStatus, OrderType, Side};
use cex_ws::route::route;
use cex_ws::wire::{Channel, Payload};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn batch(seq: u64, events: Vec<Event>) -> EventBatch {
    EventBatch {
        seq,
        request_id: Uuid::new_v4(),
        events,
    }
}

fn fill(maker: Uuid, taker: Uuid) -> Fill {
    Fill {
        symbol: SYM.into(),
        price: P50K,
        qty: Q1,
        maker_order_id: 1,
        taker_order_id: 2,
        maker_user_id: maker,
        taker_user_id: taker,
        taker_side: Side::Buy,
        notional: 50_000_000,
        maker_fee: 10_000,
        taker_fee: 50,
    }
}

// ───────────────────────── public market data ─────────────────────────

#[tokio::test]
async fn a_depth_update_goes_to_the_symbols_depth_channel() {
    let updates = route(&batch(
        3,
        vec![Event::DepthUpdated {
            symbol: SYM.into(),
            depth_seq: 9,
            deltas: vec![DepthDelta {
                side: Side::Buy,
                price: P50K,
                qty: 0,
            }],
        }],
    ));

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].channel, Channel::Depth(SYM.into()));
    assert_eq!(updates[0].audience, None, "depth is public");
    assert_eq!(updates[0].seq, 3);

    let env: cex_ws::Envelope = serde_json::from_str(&updates[0].payload).unwrap();
    assert_eq!(env.channel, "depth@BTC_USDT");
    match env.data {
        Payload::Depth(d) => {
            assert_eq!(d.depth_seq, 9);
            assert_eq!(d.deltas.len(), 1);
        }
        other => panic!("expected depth, got {other:?}"),
    }
}

#[tokio::test]
async fn a_trade_prints_on_the_public_trades_channel() {
    let updates = route(&batch(
        4,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![fill(Uuid::new_v4(), Uuid::new_v4())],
        }],
    ));

    let public: Vec<_> = updates
        .iter()
        .filter(|u| u.channel == Channel::Trades(SYM.into()))
        .collect();
    assert_eq!(public.len(), 1);
    assert_eq!(public[0].audience, None);

    let env: cex_ws::Envelope = serde_json::from_str(&public[0].payload).unwrap();
    match env.data {
        Payload::Trade(t) => {
            assert_eq!(t.price, P50K);
            assert_eq!(t.qty, Q1);
            assert_eq!(t.taker_side, Side::Buy);
        }
        other => panic!("expected trade, got {other:?}"),
    }
}

/// The one that matters. A `Fill` names both counterparties; the public feed
/// must not.
#[tokio::test]
async fn the_public_trade_feed_never_carries_a_user_id() {
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    let updates = route(&batch(
        4,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![fill(maker, taker)],
        }],
    ));

    for update in updates.iter().filter(|u| u.audience.is_none()) {
        let json = &update.payload;
        assert!(
            !json.contains(&maker.to_string()),
            "public payload leaked the maker: {json}"
        );
        assert!(
            !json.contains(&taker.to_string()),
            "public payload leaked the taker: {json}"
        );
    }
}

#[tokio::test]
async fn each_fill_in_a_sweep_prints_separately() {
    let taker = Uuid::new_v4();
    let mut second = fill(Uuid::new_v4(), taker);
    second.price = P50K + 1_000_000;

    let updates = route(&batch(
        4,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![fill(Uuid::new_v4(), taker), second],
        }],
    ));

    let prints = updates
        .iter()
        .filter(|u| u.channel == Channel::Trades(SYM.into()))
        .count();
    assert_eq!(prints, 2, "a taker sweeping two makers printed two trades");
}

// ───────────────────────── the private feed ─────────────────────────

#[tokio::test]
async fn a_fill_reaches_both_sides_privately_and_only_them() {
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    let updates = route(&batch(
        4,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![fill(maker, taker)],
        }],
    ));

    let private: Vec<_> = updates
        .iter()
        .filter(|u| u.channel == Channel::Orders)
        .collect();
    assert_eq!(private.len(), 2, "one for the maker, one for the taker");

    let audiences: Vec<_> = private.iter().filter_map(|u| u.audience).collect();
    assert!(audiences.contains(&maker));
    assert!(audiences.contains(&taker));
    assert!(
        private.iter().all(|u| u.audience.is_some()),
        "a private update must always name its audience"
    );
}

#[tokio::test]
async fn each_side_of_a_fill_is_told_its_own_role_side_and_fee() {
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    let updates = route(&batch(
        4,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![fill(maker, taker)],
        }],
    ));

    let decode = |who: Uuid| -> cex_ws::wire::OrderUpdate {
        let u = updates
            .iter()
            .find(|u| u.audience == Some(who) && u.channel == Channel::Orders)
            .expect("an update for this user");
        let env: cex_ws::Envelope = serde_json::from_str(&u.payload).unwrap();
        match env.data {
            Payload::Order(o) => o,
            other => panic!("expected order, got {other:?}"),
        }
    };

    match decode(maker) {
        cex_ws::wire::OrderUpdate::Fill {
            order_id,
            side,
            fee,
            role,
            ..
        } => {
            assert_eq!(order_id, 1, "the maker hears about its own order");
            // The taker bought, so the maker was the seller.
            assert_eq!(side, Side::Sell);
            assert_eq!(fee, 10_000, "the maker fee, not the taker's");
            assert_eq!(role, cex_ws::wire::Role::Maker);
        }
        other => panic!("expected a fill, got {other:?}"),
    }

    match decode(taker) {
        cex_ws::wire::OrderUpdate::Fill {
            order_id,
            side,
            fee,
            role,
            ..
        } => {
            assert_eq!(order_id, 2);
            assert_eq!(side, Side::Buy);
            assert_eq!(fee, 50, "the taker fee, not the maker's");
            assert_eq!(role, cex_ws::wire::Role::Taker);
        }
        other => panic!("expected a fill, got {other:?}"),
    }
}

/// `seq` alone does not identify a fill — one command can produce several, and
/// a client that deduplicates on `seq` would throw away all but the first.
/// `idx` counts across the whole batch rather than restarting inside each
/// event, so that `(seq, idx)` is the same identity the `fills` table is keyed
/// on and a live update can be matched against the history row it becomes.
#[tokio::test]
async fn a_private_fill_carries_its_index_within_the_batch() {
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    let updates = route(&batch(
        7,
        vec![
            Event::Trades {
                symbol: SYM.into(),
                fills: vec![fill(maker, taker), fill(maker, taker)],
            },
            Event::Trades {
                symbol: SYM.into(),
                fills: vec![fill(maker, taker)],
            },
        ],
    ));

    let indices: Vec<i32> = updates
        .iter()
        .filter(|u| u.audience == Some(taker))
        .map(|u| {
            let env: cex_ws::Envelope = serde_json::from_str(&u.payload).unwrap();
            match env.data {
                Payload::Order(cex_ws::wire::OrderUpdate::Fill { idx, .. }) => idx,
                other => panic!("expected a fill, got {other:?}"),
            }
        })
        .collect();

    assert_eq!(
        indices,
        vec![0, 1, 2],
        "the counter runs across the batch, not per event"
    );
}

#[tokio::test]
async fn a_private_fill_does_not_name_the_counterparty() {
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    let updates = route(&batch(
        4,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![fill(maker, taker)],
        }],
    ));

    let to_maker = updates
        .iter()
        .find(|u| u.audience == Some(maker))
        .expect("an update for the maker");
    assert!(
        !to_maker.payload.contains(&taker.to_string()),
        "the maker was told who it traded with: {}",
        to_maker.payload
    );

    let to_taker = updates
        .iter()
        .find(|u| u.audience == Some(taker))
        .expect("an update for the taker");
    assert!(
        !to_taker.payload.contains(&maker.to_string()),
        "the taker was told who it traded with: {}",
        to_taker.payload
    );
}

#[tokio::test]
async fn order_lifecycle_events_are_private_to_their_owner() {
    let alice = Uuid::new_v4();

    let updates = route(&batch(
        1,
        vec![
            Event::OrderAccepted {
                order_id: 7,
                user_id: alice,
                symbol: SYM.into(),
                side: Side::Buy,
                order_type: OrderType::Limit,
                price: Some(P50K),
                qty: Q1,
            },
            Event::OrderUpdated {
                order_id: 7,
                user_id: alice,
                filled_qty: 0,
                qty: Q1,
                status: OrderStatus::Open,
            },
            Event::OrderCancelled {
                order_id: 7,
                user_id: alice,
                symbol: SYM.into(),
                unfilled_qty: Q1,
            },
            Event::OrderRejected {
                user_id: alice,
                symbol: SYM.into(),
                reason: "below min notional".into(),
            },
        ],
    ));

    assert_eq!(updates.len(), 4);
    assert!(updates.iter().all(|u| u.channel == Channel::Orders));
    assert!(updates.iter().all(|u| u.audience == Some(alice)));
}

// ───────────────────────── events with nothing to broadcast ─────────────────────────

#[tokio::test]
async fn balance_events_produce_no_updates() {
    // Balances are not one of the three channels. Routing them to `orders`
    // anyway would send clients a message shape they never subscribed to.
    let updates = route(&batch(
        1,
        vec![
            Event::Deposited {
                user_id: Uuid::new_v4(),
                asset: "USDT".into(),
                amount: 1_000,
                available: 1_000,
            },
            Event::Withdrawn {
                user_id: Uuid::new_v4(),
                asset: "USDT".into(),
                amount: 500,
                available: 500,
            },
            Event::BalanceUpdated {
                user_id: Uuid::new_v4(),
                asset: "BTC".into(),
                available: 1,
                locked: 0,
            },
        ],
    ));

    assert!(updates.is_empty());
}

#[tokio::test]
async fn every_update_carries_the_batch_seq() {
    let alice = Uuid::new_v4();
    let updates = route(&batch(
        42,
        vec![
            Event::OrderAccepted {
                order_id: 7,
                user_id: alice,
                symbol: SYM.into(),
                side: Side::Buy,
                order_type: OrderType::Limit,
                price: Some(P50K),
                qty: Q1,
            },
            Event::DepthUpdated {
                symbol: SYM.into(),
                depth_seq: 1,
                deltas: vec![],
            },
        ],
    ));

    assert!(!updates.is_empty());
    assert!(updates.iter().all(|u| u.seq == 42));
}

// ───────────────────────── channel names ─────────────────────────

#[tokio::test]
async fn channel_names_round_trip() {
    for name in ["depth@BTC_USDT", "trades@ETH_USDT", "orders"] {
        let parsed: Channel = name.parse().unwrap();
        assert_eq!(parsed.to_string(), name);
    }
}

#[tokio::test]
async fn nonsense_channel_names_are_refused() {
    for name in [
        "",
        "depth",
        "depth@",
        "book@BTC_USDT",
        "ORDERS",
        "@BTC_USDT",
    ] {
        assert!(
            name.parse::<Channel>().is_err(),
            "{name:?} should not parse"
        );
    }
}

#[tokio::test]
async fn only_the_orders_channel_is_private() {
    assert!(Channel::Orders.is_private());
    assert!(!Channel::Depth(SYM.into()).is_private());
    assert!(!Channel::Trades(SYM.into()).is_private());
}
