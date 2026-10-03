//! History retention deletes old rows in batches.

use cex_persist::test_support::age_all_batches;
use cex_persist::{HistoryStore, TestResources};
use cex_proto::{Event, EventBatch, OrderType, Side};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";

#[tokio::test]
async fn old_rows_are_deleted_in_batches() {
    let resources = TestResources::new();
    let database_url = std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into());
    let store = HistoryStore::connect_to_schema(&database_url, &resources.schema)
        .await
        .unwrap();

    let user = Uuid::new_v4();
    store
        .write_batches(&[EventBatch {
            seq: 1,
            request_id: Uuid::new_v4(),
            events: vec![Event::OrderAccepted {
                order_id: 1,
                user_id: user,
                symbol: SYM.into(),
                side: Side::Buy,
                order_type: OrderType::Limit,
                price: Some(1),
                qty: 1,
            }],
        }])
        .await
        .unwrap();

    age_all_batches(&database_url, &resources.schema, 30)
        .await
        .unwrap();

    let deleted = store.delete_history_older_than(14, 5_000).await.unwrap();
    assert!(deleted >= 1);
    assert!(store.written_seqs().await.unwrap().is_empty());
}
