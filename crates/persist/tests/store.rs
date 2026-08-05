//! The history tables, against a real Postgres.
//!
//! These need the compose stack up (`docker compose up -d`). Each test gets its
//! own schema, so they neither collide with each other nor need tearing down.
//!
//! The two that matter are `a_failed_statement_leaves_nothing_from_the_batch_behind`
//! — a crash mid-batch must leave no half-written history — and
//! `redelivering_the_same_batch_writes_no_duplicate_rows`, because redelivery is
//! not a hypothetical: the engine republishes events every time it recovers.

use cex_persist::HistoryStore;
use cex_proto::{DepthDelta, Event, EventBatch, Fill, OrderStatus, OrderType, Side};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn database_url() -> String {
    std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into())
}

async fn store() -> HistoryStore {
    let schema = format!("t{}", Uuid::new_v4().simple());
    HistoryStore::connect_to_schema(&database_url(), &schema)
        .await
        .expect("postgres — is `docker compose up -d` running?")
}

fn accepted(order_id: u64, user: Uuid, side: Side, price: i64, qty: i64) -> Event {
    Event::OrderAccepted {
        order_id,
        user_id: user,
        symbol: SYM.into(),
        side,
        order_type: OrderType::Limit,
        price: Some(price),
        qty,
    }
}

fn updated(order_id: u64, user: Uuid, filled_qty: i64, qty: i64, status: OrderStatus) -> Event {
    Event::OrderUpdated {
        order_id,
        user_id: user,
        filled_qty,
        qty,
        status,
    }
}

fn fill(maker_order: u64, taker_order: u64, maker: Uuid, taker: Uuid, qty: i64) -> Fill {
    Fill {
        symbol: SYM.into(),
        price: P50K,
        qty,
        maker_order_id: maker_order,
        taker_order_id: taker_order,
        maker_user_id: maker,
        taker_user_id: taker,
        taker_side: Side::Buy,
        notional: 50_000_000,
        maker_fee: 500,
        taker_fee: 100,
    }
}

fn batch(seq: u64, events: Vec<Event>) -> EventBatch {
    EventBatch {
        seq,
        request_id: Uuid::new_v4(),
        events,
    }
}

// ───────────────────────── the batch guard ─────────────────────────

#[tokio::test]
async fn a_batch_is_recorded_by_seq() {
    let s = store().await;
    let alice = Uuid::new_v4();

    s.write_batches(&[
        batch(1, vec![accepted(1, alice, Side::Buy, P50K, Q1)]),
        batch(2, vec![accepted(2, alice, Side::Sell, P50K, Q1)]),
    ])
    .await
    .unwrap();

    assert_eq!(s.written_seqs().await.unwrap(), vec![1, 2]);
}

#[tokio::test]
async fn redelivering_the_same_batch_writes_no_duplicate_rows() {
    let s = store().await;
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    let batches = vec![
        batch(1, vec![accepted(1, alice, Side::Sell, P50K, Q1)]),
        batch(
            2,
            vec![
                accepted(2, bob, Side::Buy, P50K, Q1),
                Event::Trades {
                    symbol: SYM.into(),
                    fills: vec![fill(1, 2, alice, bob, Q1)],
                },
                updated(1, alice, Q1, Q1, OrderStatus::Filled),
                updated(2, bob, Q1, Q1, OrderStatus::Filled),
                Event::BalanceUpdated {
                    user_id: bob,
                    asset: "BTC".into(),
                    available: Q1,
                    locked: 0,
                },
            ],
        ),
    ];

    let first = s.write_batches(&batches).await.unwrap();
    assert_eq!(first, 2, "both batches are new the first time");

    // Exactly what the engine does after a restart: the same seqs, published again.
    let second = s.write_batches(&batches).await.unwrap();
    assert_eq!(second, 0, "neither batch is new the second time");

    assert_eq!(s.written_seqs().await.unwrap(), vec![1, 2]);
    assert_eq!(
        s.fills_for_symbol(SYM, 100).await.unwrap().len(),
        1,
        "the fill was written once, not twice"
    );
    assert_eq!(
        s.balance_changes_for(bob).await.unwrap().len(),
        1,
        "the balance change was written once, not twice"
    );
}

#[tokio::test]
async fn a_partially_redelivered_run_writes_only_the_new_batches() {
    let s = store().await;
    let alice = Uuid::new_v4();

    s.write_batches(&[batch(1, vec![accepted(1, alice, Side::Buy, P50K, Q1)])])
        .await
        .unwrap();

    // Redis redelivered batch 1 alongside a genuinely new batch 2.
    let written = s
        .write_batches(&[
            batch(1, vec![accepted(1, alice, Side::Buy, P50K, Q1)]),
            batch(2, vec![accepted(2, alice, Side::Sell, P50K, Q1)]),
        ])
        .await
        .unwrap();

    assert_eq!(written, 1, "only batch 2 was new");
    assert_eq!(s.written_seqs().await.unwrap(), vec![1, 2]);
    assert!(s.order(2).await.unwrap().is_some());
}

// ───────────────────────── atomicity ─────────────────────────

#[tokio::test]
async fn a_failed_statement_leaves_nothing_from_the_batch_behind() {
    let s = store().await;
    let alice = Uuid::new_v4();

    // A batch whose first event writes cleanly and whose second violates the
    // `qty > 0` check. If the write were not one transaction, the deposit row
    // would survive the failure.
    let poisoned = batch(
        1,
        vec![
            Event::Deposited {
                user_id: alice,
                asset: "USDT".into(),
                amount: 1_000,
                available: 1_000,
            },
            accepted(1, alice, Side::Buy, P50K, 0),
        ],
    );

    let err = s.write_batches(&[poisoned]).await;
    assert!(err.is_err(), "the batch must not be reported as written");

    assert!(
        s.written_seqs().await.unwrap().is_empty(),
        "no batch was recorded"
    );
    assert!(
        s.balance_changes_for(alice).await.unwrap().is_empty(),
        "the deposit written before the failure was rolled back"
    );
    assert!(s.order(1).await.unwrap().is_none());
}

#[tokio::test]
async fn a_later_batch_failing_rolls_back_the_earlier_ones_too() {
    let s = store().await;
    let alice = Uuid::new_v4();

    let err = s
        .write_batches(&[
            batch(1, vec![accepted(1, alice, Side::Buy, P50K, Q1)]),
            batch(2, vec![accepted(2, alice, Side::Sell, P50K, -5)]),
        ])
        .await;

    assert!(err.is_err());
    assert!(
        s.written_seqs().await.unwrap().is_empty(),
        "the good batch went back too — all or nothing, so a retry starts clean"
    );
    assert!(s.order(1).await.unwrap().is_none());
}

// ───────────────────────── the order lifecycle ─────────────────────────

#[tokio::test]
async fn an_order_row_follows_its_lifecycle_to_filled() {
    let s = store().await;
    let alice = Uuid::new_v4();

    s.write_batches(&[batch(1, vec![accepted(7, alice, Side::Buy, P50K, Q1)])])
        .await
        .unwrap();

    let row = s.order(7).await.unwrap().unwrap();
    assert_eq!(row.user_id, alice);
    assert_eq!(row.symbol, SYM);
    assert_eq!(row.side, "BUY");
    assert_eq!(row.order_type, "LIMIT");
    assert_eq!(row.price, Some(P50K));
    assert_eq!(row.qty, Q1);
    assert_eq!(row.filled_qty, 0);
    assert_eq!(row.status, "OPEN");

    s.write_batches(&[batch(
        2,
        vec![updated(7, alice, Q1 / 2, Q1, OrderStatus::PartiallyFilled)],
    )])
    .await
    .unwrap();
    let row = s.order(7).await.unwrap().unwrap();
    assert_eq!(row.filled_qty, Q1 / 2);
    assert_eq!(row.status, "PARTIALLY_FILLED");

    s.write_batches(&[batch(
        3,
        vec![updated(7, alice, Q1, Q1, OrderStatus::Filled)],
    )])
    .await
    .unwrap();
    let row = s.order(7).await.unwrap().unwrap();
    assert_eq!(row.filled_qty, Q1);
    assert_eq!(row.status, "FILLED");
    assert_eq!(row.last_seq, 3);
}

#[tokio::test]
async fn a_cancel_marks_the_order_cancelled() {
    let s = store().await;
    let alice = Uuid::new_v4();

    s.write_batches(&[
        batch(1, vec![accepted(7, alice, Side::Buy, P50K, Q1)]),
        batch(
            2,
            vec![Event::OrderCancelled {
                order_id: 7,
                user_id: alice,
                symbol: SYM.into(),
                unfilled_qty: Q1,
            }],
        ),
    ])
    .await
    .unwrap();

    assert_eq!(s.order(7).await.unwrap().unwrap().status, "CANCELLED");
}

#[tokio::test]
async fn a_market_order_is_stored_with_no_price() {
    let s = store().await;
    let alice = Uuid::new_v4();

    s.write_batches(&[batch(
        1,
        vec![Event::OrderAccepted {
            order_id: 9,
            user_id: alice,
            symbol: SYM.into(),
            side: Side::Buy,
            order_type: OrderType::Market,
            price: None,
            qty: Q1,
        }],
    )])
    .await
    .unwrap();

    let row = s.order(9).await.unwrap().unwrap();
    assert_eq!(row.order_type, "MARKET");
    assert_eq!(row.price, None);
}

#[tokio::test]
async fn an_older_update_does_not_overwrite_a_newer_one() {
    let s = store().await;
    let alice = Uuid::new_v4();

    s.write_batches(&[
        batch(1, vec![accepted(7, alice, Side::Buy, P50K, Q1)]),
        batch(9, vec![updated(7, alice, Q1, Q1, OrderStatus::Filled)]),
    ])
    .await
    .unwrap();

    // Seq 5 is older than the seq 9 already recorded on the row. Applying it
    // would resurrect a filled order as partially filled.
    s.write_batches(&[batch(
        5,
        vec![updated(7, alice, Q1 / 2, Q1, OrderStatus::PartiallyFilled)],
    )])
    .await
    .unwrap();

    let row = s.order(7).await.unwrap().unwrap();
    assert_eq!(row.status, "FILLED", "the newer state survived");
    assert_eq!(row.filled_qty, Q1);
    assert_eq!(row.last_seq, 9);
}

// ───────────────────────── fills ─────────────────────────

#[tokio::test]
async fn a_fill_records_both_sides_and_both_fees() {
    let s = store().await;
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    s.write_batches(&[batch(
        4,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![fill(1, 2, maker, taker, Q1)],
        }],
    )])
    .await
    .unwrap();

    let rows = s.fills_for_symbol(SYM, 100).await.unwrap();
    assert_eq!(rows.len(), 1);
    let f = &rows[0];
    assert_eq!(f.seq, 4);
    assert_eq!(f.price, P50K);
    assert_eq!(f.qty, Q1);
    assert_eq!(f.maker_order_id, 1);
    assert_eq!(f.taker_order_id, 2);
    assert_eq!(f.maker_user_id, maker);
    assert_eq!(f.taker_user_id, taker);
    assert_eq!(f.taker_side, "BUY");
    assert_eq!(f.notional, 50_000_000);
    assert_eq!(f.maker_fee, 500);
    assert_eq!(f.taker_fee, 100);
}

#[tokio::test]
async fn several_fills_in_one_batch_all_survive() {
    let s = store().await;
    let maker_a = Uuid::new_v4();
    let maker_b = Uuid::new_v4();
    let taker = Uuid::new_v4();

    // One taker sweeping two makers: same seq, so the rows can only be told
    // apart by their index within the batch.
    s.write_batches(&[batch(
        4,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![
                fill(1, 3, maker_a, taker, Q1),
                fill(2, 3, maker_b, taker, Q1 * 2),
            ],
        }],
    )])
    .await
    .unwrap();

    let rows = s.fills_for_symbol(SYM, 100).await.unwrap();
    assert_eq!(rows.len(), 2, "neither fill overwrote the other");
    assert_eq!(rows.iter().map(|f| f.qty).sum::<i64>(), Q1 * 3);
    assert_ne!(rows[0].idx, rows[1].idx);
}

#[tokio::test]
async fn fills_are_only_returned_for_the_symbol_asked_for() {
    let s = store().await;
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    let mut other = fill(1, 2, maker, taker, Q1);
    other.symbol = "ETH_USDT".into();

    s.write_batches(&[batch(
        1,
        vec![
            Event::Trades {
                symbol: SYM.into(),
                fills: vec![fill(1, 2, maker, taker, Q1)],
            },
            Event::Trades {
                symbol: "ETH_USDT".into(),
                fills: vec![other],
            },
        ],
    )])
    .await
    .unwrap();

    let rows = s.fills_for_symbol(SYM, 100).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, SYM);
}

// ───────────────────────── balance changes ─────────────────────────

#[tokio::test]
async fn a_deposit_and_a_withdrawal_are_recorded_with_signed_deltas() {
    let s = store().await;
    let alice = Uuid::new_v4();

    s.write_batches(&[
        batch(
            1,
            vec![Event::Deposited {
                user_id: alice,
                asset: "USDT".into(),
                amount: 1_000,
                available: 1_000,
            }],
        ),
        batch(
            2,
            vec![Event::Withdrawn {
                user_id: alice,
                asset: "USDT".into(),
                amount: 400,
                available: 600,
            }],
        ),
    ])
    .await
    .unwrap();

    let rows = s.balance_changes_for(alice).await.unwrap();
    assert_eq!(rows.len(), 2);

    let deposit = rows.iter().find(|r| r.seq == 1).unwrap();
    assert_eq!(deposit.reason, "deposit");
    assert_eq!(deposit.delta, Some(1_000));
    assert_eq!(deposit.available, 1_000);

    let withdrawal = rows.iter().find(|r| r.seq == 2).unwrap();
    assert_eq!(withdrawal.reason, "withdrawal");
    assert_eq!(
        withdrawal.delta,
        Some(-400),
        "a withdrawal leaves the account, so its delta is negative"
    );
    assert_eq!(withdrawal.available, 600);
}

#[tokio::test]
async fn a_settlement_balance_update_records_available_and_locked() {
    let s = store().await;
    let alice = Uuid::new_v4();

    s.write_batches(&[batch(
        1,
        vec![Event::BalanceUpdated {
            user_id: alice,
            asset: "BTC".into(),
            available: 700,
            locked: 300,
        }],
    )])
    .await
    .unwrap();

    let rows = s.balance_changes_for(alice).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reason, "update");
    assert_eq!(rows[0].available, 700);
    assert_eq!(rows[0].locked, Some(300));
    assert_eq!(
        rows[0].delta, None,
        "the event stated a resulting balance, not a movement"
    );
}

#[tokio::test]
async fn one_users_balance_changes_do_not_include_anothers() {
    let s = store().await;
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    s.write_batches(&[batch(
        1,
        vec![
            Event::BalanceUpdated {
                user_id: alice,
                asset: "BTC".into(),
                available: 1,
                locked: 0,
            },
            Event::BalanceUpdated {
                user_id: bob,
                asset: "BTC".into(),
                available: 2,
                locked: 0,
            },
        ],
    )])
    .await
    .unwrap();

    let rows = s.balance_changes_for(alice).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].available, 1);
}

// ───────────────────────── events with no history to write ─────────────────────────

#[tokio::test]
async fn depth_updates_are_not_history_and_write_nothing() {
    let s = store().await;

    // Depth is live market data — `ws` fans it out, it is not something to
    // store. The batch must still be recorded so it is never reprocessed.
    s.write_batches(&[batch(
        1,
        vec![Event::DepthUpdated {
            symbol: SYM.into(),
            depth_seq: 4,
            deltas: vec![DepthDelta {
                side: Side::Buy,
                price: P50K,
                qty: 0,
            }],
        }],
    )])
    .await
    .unwrap();

    assert_eq!(s.written_seqs().await.unwrap(), vec![1]);
    assert!(s.fills_for_symbol(SYM, 100).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_rejected_order_records_the_batch_but_no_order_row() {
    let s = store().await;
    let alice = Uuid::new_v4();

    // A rejection carries no order id, so there is nothing to key a row on.
    s.write_batches(&[batch(
        1,
        vec![Event::OrderRejected {
            user_id: alice,
            symbol: SYM.into(),
            reason: "below min notional".into(),
        }],
    )])
    .await
    .unwrap();

    assert_eq!(s.written_seqs().await.unwrap(), vec![1]);
}

#[tokio::test]
async fn writing_no_batches_at_all_is_not_an_error() {
    let s = store().await;
    assert_eq!(s.write_batches(&[]).await.unwrap(), 0);
    assert!(s.written_seqs().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_schema_name_that_could_terminate_a_statement_is_refused() {
    // The schema name is spliced into `CREATE SCHEMA`, which cannot take a
    // bound parameter. It is validated instead.
    let bad = HistoryStore::connect_to_schema(&database_url(), "x\"; DROP TABLE users; --").await;
    assert!(bad.is_err());
}
