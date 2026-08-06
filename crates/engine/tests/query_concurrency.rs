//! Proves reads no longer wait behind the blocking command-stream read.
//!
//! These drive `Runner::run()` for real (not `step()`/`poll_queries()` by hand), because the
//! bug and the fix both live in how `run()` interleaves the two loops.

use std::collections::HashSet;
use std::time::Duration;

use cex_engine::config::Config;
use cex_engine::runner::Runner;
use cex_proto::{Command, Query, Response, ResponseBody, ResponseResult, FIELD_PAYLOAD};
use futures_util::StreamExt;
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

fn deposit(who: Uuid, asset: &str, amount: i64) -> Command {
    Command::Deposit {
        request_id: Uuid::new_v4(),
        user_id: who,
        asset: asset.into(),
        amount,
    }
}

async fn send_command(c: &mut redis::aio::MultiplexedConnection, cfg: &Config, cmd: &Command) {
    let json = serde_json::to_string(cmd).unwrap();
    let _: String = c
        .xadd(&cfg.commands_stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
        .await
        .unwrap();
}

/// Waits on `sub` for a response whose `request_id` is `want`, ignoring anything else on the
/// shared response channel — exactly what a real caller's routing table does.
async fn wait_for(sub: &mut redis::aio::PubSub, want: Uuid, deadline: Duration) -> Response {
    tokio::time::timeout(deadline, async {
        let mut stream = sub.on_message();
        loop {
            let msg = stream.next().await.expect("subscription closed");
            let payload: String = msg.get_payload().unwrap();
            let r: Response = serde_json::from_str(&payload).unwrap();
            if r.request_id == want {
                return r;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no response for {want} within {deadline:?}"))
}

/// Catches the "query task holds a stale clone of state instead of the shared `Arc`" mistake: if
/// it did, this query would see the pre-deposit balance (empty) forever, because the clone was
/// taken once at spawn time and the two states would have diverged from then on.
#[tokio::test]
async fn a_query_after_run_starts_reflects_a_command_sent_after_run_starts() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let alice = Uuid::new_v4();

    let runner = Runner::boot(cfg.clone()).await.unwrap();
    let run_task = tokio::spawn(async move {
        let mut runner = runner;
        let _ = runner.run().await;
    });

    // Send the command and wait for its own ack — the same thing a real caller
    // (the API's `Loopback`) does before it can know the deposit landed.
    let cmd = deposit(alice, "USDT", 1_234);
    let deposit_rid = cmd.request_id();
    send_command(&mut c, &cfg, &cmd).await;
    let ack = wait_for(&mut sub, deposit_rid, Duration::from_secs(3)).await;
    assert!(matches!(ack.result, ResponseResult::Ok { .. }));

    // Now query. The deposit is durably applied by this point, so the read must
    // reflect it.
    let query_rid = Uuid::new_v4();
    let query_json = serde_json::to_string(&Query::Balances {
        request_id: query_rid,
        user_id: alice,
    })
    .unwrap();
    let _: i64 = c.lpush(&cfg.queries_queue, query_json).await.unwrap();
    let answer = wait_for(&mut sub, query_rid, Duration::from_secs(3)).await;

    match answer.result {
        ResponseResult::Ok {
            data: ResponseBody::Balances(v),
        } => {
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].asset, "USDT");
            assert_eq!(v[0].available, 1_234, "query did not see the acked deposit");
        }
        other => panic!("expected an Ok Balances response, got {other:?}"),
    }

    run_task.abort();
}

/// Fires a burst of deposits and a burst of interleaved balance queries at the same time, through
/// the real concurrent `run()` loop, then checks the final balance is exactly the sum of every
/// deposit — nothing lost, nothing double-counted. This is the closest an external test gets to
/// proving the lock actually serializes commands against each other and against queries under
/// real contention.
#[tokio::test]
async fn commands_and_queries_interleaved_under_load_land_on_the_exact_expected_total() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut cmd_conn = conn(&cfg).await;
    let mut query_conn = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let alice = Uuid::new_v4();

    let runner = Runner::boot(cfg.clone()).await.unwrap();
    let run_task = tokio::spawn(async move {
        let mut runner = runner;
        let _ = runner.run().await;
    });

    const N: usize = 200;
    let mut pending: HashSet<Uuid> = HashSet::with_capacity(N);
    for _ in 0..N {
        let cmd = deposit(alice, "USDT", 10);
        pending.insert(cmd.request_id());
        send_command(&mut cmd_conn, &cfg, &cmd).await;

        // Interleave a query for every command sent — best effort, not
        // individually asserted on, but real concurrent load on the same
        // shared state the commands are mutating.
        let query_json = serde_json::to_string(&Query::Balances {
            request_id: Uuid::new_v4(),
            user_id: alice,
        })
        .unwrap();
        let _: i64 = query_conn.lpush(&cfg.queries_queue, query_json).await.unwrap();
    }

    // Wait for every deposit to be acked before asking for the final total —
    // exactly what makes the final read's expectation well-defined.
    let deadline = Duration::from_secs(10);
    let mut stream = sub.on_message();
    let started = std::time::Instant::now();
    while !pending.is_empty() {
        assert!(started.elapsed() < deadline, "commands did not all ack in time");
        let msg = tokio::time::timeout(deadline, stream.next())
            .await
            .expect("timed out waiting for acks")
            .expect("subscription closed");
        let payload: String = msg.get_payload().unwrap();
        let r: Response = serde_json::from_str(&payload).unwrap();
        pending.remove(&r.request_id);
    }
    drop(stream);

    let query_rid = Uuid::new_v4();
    let query_json = serde_json::to_string(&Query::Balances {
        request_id: query_rid,
        user_id: alice,
    })
    .unwrap();
    let _: i64 = query_conn.lpush(&cfg.queries_queue, query_json).await.unwrap();
    let answer = wait_for(&mut sub, query_rid, Duration::from_secs(5)).await;

    match answer.result {
        ResponseResult::Ok {
            data: ResponseBody::Balances(v),
        } => {
            assert_eq!(v.len(), 1);
            assert_eq!(
                v[0].available,
                10 * N as i64,
                "final balance does not match the sum of every deposit — a command was lost, \
                 double-applied, or a query observed a torn state"
            );
        }
        other => panic!("expected an Ok Balances response, got {other:?}"),
    }

    run_task.abort();
}
