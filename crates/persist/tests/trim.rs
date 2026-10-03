//! Stream trimming for confirmed events.

use cex_persist::{trim, TestResources};
use cex_proto::{Event, EventBatch, FIELD_PAYLOAD};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use uuid::Uuid;

const GROUP_A: &str = "trim-test-a";
const GROUP_B: &str = "trim-test-b";

async fn conn(redis_url: &str) -> redis::aio::MultiplexedConnection {
    redis::Client::open(redis_url)
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

#[tokio::test]
async fn events_are_trimmed_only_after_both_groups_confirm() {
    let resources = TestResources::new();
    let redis_url =
        std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into());
    let stream = format!("test:{}:events", resources.tag);
    let mut r = conn(&redis_url).await;

    let _: String = r
        .xgroup_create_mkstream(&stream, GROUP_A, "0")
        .await
        .unwrap();
    let _: String = r
        .xgroup_create_mkstream(&stream, GROUP_B, "0")
        .await
        .unwrap();

    let batch = EventBatch {
        seq: 1,
        request_id: Uuid::new_v4(),
        events: vec![Event::DepthUpdated {
            symbol: "BTC_USDT".into(),
            depth_seq: 1,
            deltas: vec![],
        }],
    };
    let json = serde_json::to_string(&batch).unwrap();
    let _: String = r
        .xadd(&stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
        .await
        .unwrap();

    for group in [GROUP_A, GROUP_B] {
        let opts = StreamReadOptions::default().group(group, "c1").count(1);
        let reply: Option<StreamReadReply> =
            r.xread_options(&[&stream], &[">"], &opts).await.unwrap();
        let ids: Vec<String> = reply
            .unwrap()
            .keys
            .iter()
            .flat_map(|k| k.ids.iter().map(|e| e.id.clone()))
            .collect();
        let _: i64 = r.xack(&stream, group, &ids).await.unwrap();
    }

    let mut manager = redis::aio::ConnectionManager::new(redis::Client::open(redis_url).unwrap())
        .await
        .unwrap();
    let removed = trim::trim_confirmed_events(&mut manager, &stream, &[GROUP_A, GROUP_B])
        .await
        .unwrap();
    assert!(removed >= 1, "confirmed entry should be trimmed");

    let len: usize = redis::cmd("XLEN")
        .arg(&stream)
        .query_async(&mut r)
        .await
        .unwrap();
    assert_eq!(len, 0, "stream should be empty after trim");
}
