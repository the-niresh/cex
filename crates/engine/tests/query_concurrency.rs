//! Proves reads no longer wait behind the blocking command-stream read.
//!
//! These drive `Runner::run()` for real (not `step()`/`poll_queries()` by hand), because the
//! bug and the fix both live in how `run()` interleaves the two loops.

use std::time::Duration;

use cex_engine::config::Config;
use cex_engine::runner::Runner;
use cex_proto::{Query, Response, ResponseResult};
use redis::AsyncCommands;
use uuid::Uuid;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

/// A high `block_ms` so a test that still waits on it fails obviously, while
/// `lock_ttl_ms` stays comfortably above the `block_ms < lock_ttl_ms / 3` floor
/// `Runner::boot` enforces.
fn test_config(dir: &std::path::Path) -> Config {
    let tag = Uuid::new_v4().simple().to_string();
    Config {
        redis_url: redis_url(),
        commands_stream: format!("test:{tag}:commands"),
        events_stream: format!("test:{tag}:events"),
        responses_channel: format!("test:{tag}:responses"),
        queries_queue: format!("test:{tag}:queries"),
        snapshot_dir: dir.to_path_buf(),
        snapshot_every: 1_000_000,
        snapshot_keep: 3,
        block_ms: 5_000,
        lock_ttl_ms: 30_000,
    }
}

async fn conn(cfg: &Config) -> redis::aio::MultiplexedConnection {
    redis::Client::open(cfg.redis_url.as_str())
        .expect("redis client")
        .get_multiplexed_async_connection()
        .await
        .expect("redis — is `docker compose up -d` running?")
}

async fn subscriber(cfg: &Config) -> redis::aio::PubSub {
    let client = redis::Client::open(cfg.redis_url.as_str()).unwrap();
    let mut sub = client.get_async_pubsub().await.unwrap();
    sub.subscribe(&cfg.responses_channel).await.unwrap();
    sub
}

#[tokio::test]
async fn a_query_is_answered_quickly_even_though_the_command_stream_is_idle_and_block_ms_is_huge()
{
    use futures_util::StreamExt;

    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;

    let runner = Runner::boot(cfg.clone()).await.unwrap();
    let run_task = tokio::spawn(async move {
        let mut runner = runner;
        let _ = runner.run().await;
    });

    // Let the command loop get past its first (empty) poll and settle into
    // `XREAD BLOCK 5000` before the query arrives — otherwise this is racy:
    // pushed early enough, the old code's first non-blocking drain could
    // happen to catch it before ever entering the blocking read.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The command stream is idle: `step()` is sitting in `XREAD BLOCK 5000`.
    let rid = Uuid::new_v4();
    let json = serde_json::to_string(&Query::Markets { request_id: rid }).unwrap();
    let _: i64 = c.lpush(&cfg.queries_queue, json).await.unwrap();

    let started = std::time::Instant::now();
    let msg = tokio::time::timeout(Duration::from_millis(800), sub.on_message().next())
        .await
        .expect("query did not answer within 800ms — it is waiting behind block_ms")
        .expect("subscription closed");
    let elapsed = started.elapsed();

    let payload: String = msg.get_payload().unwrap();
    let response: Response = serde_json::from_str(&payload).unwrap();
    assert_eq!(response.request_id, rid);
    assert!(matches!(response.result, ResponseResult::Ok { .. }));
    assert!(
        elapsed < Duration::from_millis(800),
        "query latency ({elapsed:?}) is tracking block_ms (5000ms) instead of being decoupled from it"
    );

    run_task.abort();
}
