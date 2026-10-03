//! Persist must drain its pending list after Redis comes back.

use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use cex_persist::{Consumer, TestResources};
use cex_proto::{Event, EventBatch, OrderType, Side, UserId, FIELD_PAYLOAD};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const Q1: i64 = 100_000;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6391".into())
}

fn docker_redis_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ensure_throwaway_redis() {
    let running = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", "cex-test-redis"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false);
    if running {
        return;
    }
    let exists = Command::new("docker")
        .args(["inspect", "cex-test-redis"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let status = if exists {
        Command::new("docker")
            .args(["start", "cex-test-redis"])
            .status()
            .expect("docker start")
    } else {
        let _ = Command::new("docker")
            .args(["rm", "-f", "cex-test-redis"])
            .status();
        Command::new("docker")
            .args([
                "run",
                "-d",
                "--name",
                "cex-test-redis",
                "-p",
                "6391:6379",
                "redis:7-alpine",
            ])
            .status()
            .expect("docker run")
    };
    assert!(status.success(), "could not start cex-test-redis");
}

async fn wait_for_redis() {
    for _ in 0..30 {
        if redis::Client::open(redis_url())
            .and_then(|c| c.get_connection())
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("redis did not come back");
}

fn test_config(resources: &TestResources) -> cex_persist::Config {
    let tag = &resources.tag;
    cex_persist::Config {
        redis_url: redis_url(),
        database_url: std::env::var("CEX_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into()),
        schema: resources.schema.clone(),
        events_stream: format!("test:{tag}:events"),
        group: format!("test:{tag}:group"),
        consumer: "persist-1".into(),
        count: 256,
        block_ms: 150,
    }
}

async fn conn(cfg: &cex_persist::Config) -> redis::aio::MultiplexedConnection {
    redis::Client::open(cfg.redis_url.as_str())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

fn batch(seq: u64, user: UserId) -> EventBatch {
    EventBatch {
        seq,
        request_id: Uuid::new_v4(),
        events: vec![Event::OrderAccepted {
            order_id: seq,
            user_id: user,
            symbol: SYM.into(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            price: Some(1),
            qty: Q1,
        }],
    }
}

#[tokio::test]
async fn pending_entries_survive_a_redis_restart() {
    {
        let _lock = docker_redis_lock();
        ensure_throwaway_redis();
    }
    let resources = TestResources::new();
    let cfg = test_config(&resources);
    let mut r = conn(&cfg).await;
    let alice = Uuid::new_v4();

    let store = cex_persist::HistoryStore::connect_to_schema(&cfg.database_url, &cfg.schema)
        .await
        .unwrap();
    let _ = Consumer::boot(cfg.clone(), store).await.unwrap();

    for seq in 1..=3 {
        let json = serde_json::to_string(&batch(seq, alice)).unwrap();
        let _: String = r
            .xadd(&cfg.events_stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
            .await
            .unwrap();
    }

    let opts = StreamReadOptions::default()
        .group(&cfg.group, &cfg.consumer)
        .count(256);
    let reply: Option<StreamReadReply> = r
        .xread_options(&[&cfg.events_stream], &[">"], &opts)
        .await
        .unwrap();
    assert_eq!(
        reply.unwrap().keys[0].ids.len(),
        3,
        "entries should be pending before restart"
    );

    {
        let _lock = docker_redis_lock();
        Command::new("docker")
            .args(["restart", "cex-test-redis"])
            .status()
            .expect("docker restart");
    }
    wait_for_redis().await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let store = cex_persist::HistoryStore::connect_to_schema(&cfg.database_url, &cfg.schema)
        .await
        .unwrap();
    let mut consumer = Consumer::boot(cfg, store).await.unwrap();
    loop {
        let n = consumer.step().await.expect("step after reconnect");
        if n == 0 {
            break;
        }
    }

    assert_eq!(
        consumer.store().written_seqs().await.unwrap(),
        vec![1, 2, 3]
    );
}
