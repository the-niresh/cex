//! History retention deletes old rows in batches.

use cex_persist::test_support::{age_all_batches, age_history_for_retention};
use cex_persist::{HistoryStore, TestResources};
use cex_proto::{Event, EventBatch, Fill, OrderStatus, OrderType, Side};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

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

#[tokio::test]
async fn old_rows_are_deleted_in_batches() {
    let resources = TestResources::new();
    let database_url = std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into());
    let store = HistoryStore::connect_to_schema(&database_url, &resources.schema)
        .await
        .unwrap();

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    store
        .write_batches(&[
            EventBatch {
                seq: 1,
                request_id: Uuid::new_v4(),
                events: vec![Event::OrderAccepted {
                    order_id: 1,
                    user_id: alice,
                    symbol: SYM.into(),
                    side: Side::Sell,
                    order_type: OrderType::Limit,
                    price: Some(P50K),
                    qty: Q1,
                }],
            },
            EventBatch {
                seq: 2,
                request_id: Uuid::new_v4(),
                events: vec![
                    Event::OrderAccepted {
                        order_id: 2,
                        user_id: bob,
                        symbol: SYM.into(),
                        side: Side::Buy,
                        order_type: OrderType::Limit,
                        price: Some(P50K),
                        qty: Q1,
                    },
                    Event::Trades {
                        symbol: SYM.into(),
                        fills: vec![fill(1, 2, alice, bob, Q1)],
                    },
                    Event::OrderUpdated {
                        order_id: 1,
                        user_id: alice,
                        filled_qty: Q1,
                        qty: Q1,
                        status: OrderStatus::Filled,
                    },
                    Event::BalanceUpdated {
                        user_id: bob,
                        asset: "BTC".into(),
                        available: Q1,
                        locked: 0,
                    },
                ],
            },
        ])
        .await
        .unwrap();

    age_all_batches(&database_url, &resources.schema, 30)
        .await
        .unwrap();
    age_history_for_retention(&database_url, &resources.schema, 30)
        .await
        .unwrap();

    let deleted = store.delete_history_older_than(14, 5_000).await.unwrap();
    assert!(deleted >= 1);

    assert_eq!(store.written_seqs().await.unwrap(), vec![1, 2]);
    assert_eq!(store.fills_for_symbol(SYM, 100).await.unwrap().len(), 1);
    assert!(store.order(1).await.unwrap().is_none());
    assert!(store.balance_changes_for(bob).await.unwrap().is_empty());
}
