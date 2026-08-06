//! The loopback: send a request, get the answer back.
//!
//! The API is stateless and the engine owns everything, so every write and most
//! reads are a round trip through Redis. The mechanism is a `request_id` and a
//! map of parked waiters — the same shape as the TypeScript version, with the
//! callback map replaced by `oneshot` channels.
//!
//! The property that matters most is **no crosstalk**: with hundreds of requests
//! in flight on one shared channel, each caller must receive its own answer and
//! nobody else's. `several_concurrent_requests_each_get_their_own_answer` is the
//! test that would catch a routing bug, and a routing bug here would hand one
//! user another user's balances.

use std::time::Duration;

use cex_api::loopback::{Loopback, LoopbackConfig, LoopbackError};
use cex_engine::config::Config as EngineConfig;
use cex_engine::runner::Runner;
use cex_proto::{Command, OrderType, Query, ResponseBody, Side, TimeInForce};
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

/// Matching engine + client configs pointing at the same throwaway channels.
fn pair(dir: &std::path::Path) -> (EngineConfig, LoopbackConfig) {
    let tag = Uuid::new_v4().simple().to_string();
    let commands = format!("test:{tag}:commands");
    let queries = format!("test:{tag}:queries");
    let responses = format!("test:{tag}:responses");

    (
        EngineConfig {
            redis_url: redis_url(),
            commands_stream: commands.clone(),
            events_stream: format!("test:{tag}:events"),
            responses_channel: responses.clone(),
            queries_queue: queries.clone(),
            snapshot_dir: dir.to_path_buf(),
            snapshot_every: 1_000_000,
            snapshot_keep: 3,
            block_ms: 150,
            lock_ttl_ms: 30_000,
        },
        LoopbackConfig {
            redis_url: redis_url(),
            commands_stream: commands,
            queries_queue: queries,
            responses_channel: responses,
            timeout: Duration::from_secs(5),
        },
    )
}

/// Let the engine work through everything currently queued.
async fn pump(runner: &mut Runner) {
    loop {
        let queries = runner.poll_queries().await.unwrap();
        let commands = runner.step().await.unwrap();
        if queries == 0 && commands == 0 {
            return;
        }
    }
}

/// Run the engine in the background for the duration of a test.
fn spawn_engine(mut runner: Runner) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let _ = runner.poll_queries().await;
            let _ = runner.step().await;
        }
    })
}

fn deposit(who: Uuid, asset: &str, amount: i64) -> Command {
    Command::Deposit {
        request_id: Uuid::new_v4(),
        user_id: who,
        asset: asset.into(),
        amount,
    }
}

fn limit(who: Uuid, side: Side, price: i64, qty: i64) -> Command {
    Command::PlaceOrder {
        request_id: Uuid::new_v4(),
        user_id: who,
        symbol: SYM.into(),
        side,
        order_type: OrderType::Limit,
        time_in_force: Some(TimeInForce::Gtc),
        price: Some(price),
        qty,
    }
}

// ───────────────────────── the happy path ─────────────────────────

#[tokio::test]
async fn a_command_returns_the_engines_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (ecfg, lcfg) = pair(dir.path());
    let engine = Runner::boot(ecfg).await.unwrap();
    let handle = spawn_engine(engine);
    let lb = Loopback::connect(lcfg).await.unwrap();

    let body = lb
        .command(deposit(Uuid::new_v4(), "USDT", 5_000))
        .await
        .expect("deposit should succeed");

    assert!(matches!(body, ResponseBody::Ack));
    handle.abort();
}

#[tokio::test]
async fn a_query_returns_the_engines_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (ecfg, lcfg) = pair(dir.path());
    let engine = Runner::boot(ecfg).await.unwrap();
    let handle = spawn_engine(engine);
    let lb = Loopback::connect(lcfg).await.unwrap();

    let alice = Uuid::new_v4();
    lb.command(deposit(alice, "USDT", 777)).await.unwrap();

    let body = lb
        .query(Query::Balances {
            request_id: Uuid::new_v4(),
            user_id: alice,
        })
        .await
        .unwrap();

    match body {
        ResponseBody::Balances(v) => {
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].available, 777);
        }
        other => panic!("expected Balances, got {other:?}"),
    }
    handle.abort();
}

#[tokio::test]
async fn placing_an_order_returns_its_id_and_status() {
    let dir = tempfile::tempdir().unwrap();
    let (ecfg, lcfg) = pair(dir.path());
    let engine = Runner::boot(ecfg).await.unwrap();
    let handle = spawn_engine(engine);
    let lb = Loopback::connect(lcfg).await.unwrap();

    let alice = Uuid::new_v4();
    lb.command(deposit(alice, "USDT", 1_000_000_000))
        .await
        .unwrap();
    let body = lb.command(limit(alice, Side::Buy, P50K, Q1)).await.unwrap();

    match body {
        ResponseBody::OrderPlaced {
            order_id, status, ..
        } => {
            assert!(order_id > 0);
            assert_eq!(status, cex_proto::OrderStatus::Open);
        }
        other => panic!("expected OrderPlaced, got {other:?}"),
    }
    handle.abort();
}

// ───────────────────────── errors ─────────────────────────

#[tokio::test]
async fn an_engine_rejection_surfaces_as_an_error_not_a_success() {
    let dir = tempfile::tempdir().unwrap();
    let (ecfg, lcfg) = pair(dir.path());
    let engine = Runner::boot(ecfg).await.unwrap();
    let handle = spawn_engine(engine);
    let lb = Loopback::connect(lcfg).await.unwrap();

    // No funds — the engine refuses.
    let err = lb
        .command(limit(Uuid::new_v4(), Side::Buy, P50K, Q1))
        .await
        .expect_err("an unfunded order must fail");

    match err {
        LoopbackError::Rejected(msg) => assert!(msg.contains("insufficient")),
        other => panic!("expected Rejected, got {other:?}"),
    }
    handle.abort();
}

#[tokio::test]
async fn a_request_with_nobody_listening_times_out() {
    // No engine is running, so nothing will ever answer.
    let dir = tempfile::tempdir().unwrap();
    let (_ecfg, mut lcfg) = pair(dir.path());
    lcfg.timeout = Duration::from_millis(300);
    let lb = Loopback::connect(lcfg).await.unwrap();

    let err = lb
        .query(Query::Markets {
            request_id: Uuid::new_v4(),
        })
        .await
        .expect_err("should have timed out");

    assert!(matches!(err, LoopbackError::Timeout));
}

#[tokio::test]
async fn a_timed_out_request_is_removed_from_the_pending_map() {
    // A pending entry that is never cleaned up is a memory leak that grows with
    // every failed request.
    let dir = tempfile::tempdir().unwrap();
    let (_ecfg, mut lcfg) = pair(dir.path());
    lcfg.timeout = Duration::from_millis(100);
    let lb = Loopback::connect(lcfg).await.unwrap();

    for _ in 0..5 {
        let _ = lb
            .query(Query::Markets {
                request_id: Uuid::new_v4(),
            })
            .await;
    }

    assert_eq!(lb.pending_count(), 0, "pending waiters leaked");
}

// ───────────────────────── routing ─────────────────────────

#[tokio::test]
async fn several_concurrent_requests_each_get_their_own_answer() {
    // The test that matters. Every reply travels on one shared channel, so a
    // routing mistake hands one caller another caller's data. Here that would
    // mean showing a user someone else's balance.
    let dir = tempfile::tempdir().unwrap();
    let (ecfg, lcfg) = pair(dir.path());
    let engine = Runner::boot(ecfg).await.unwrap();
    let handle = spawn_engine(engine);
    let lb = Loopback::connect(lcfg).await.unwrap();

    // Give each user a distinct, identifiable balance.
    let users: Vec<Uuid> = (0..25).map(|_| Uuid::new_v4()).collect();
    for (i, u) in users.iter().enumerate() {
        lb.command(deposit(*u, "USDT", 1_000 + i as i64))
            .await
            .unwrap();
    }

    // Ask for all of them at once.
    let futures: Vec<_> = users
        .iter()
        .map(|u| {
            let lb = &lb;
            let u = *u;
            async move {
                let body = lb
                    .query(Query::Balances {
                        request_id: Uuid::new_v4(),
                        user_id: u,
                    })
                    .await
                    .unwrap();
                match body {
                    ResponseBody::Balances(v) => v[0].available,
                    other => panic!("expected Balances, got {other:?}"),
                }
            }
        })
        .collect();

    let answers = futures_util::future::join_all(futures).await;

    for (i, got) in answers.iter().enumerate() {
        assert_eq!(
            *got,
            1_000 + i as i64,
            "caller {i} received the wrong user's balance"
        );
    }
    handle.abort();
}

#[tokio::test]
async fn a_reply_for_an_unknown_request_is_ignored() {
    // Late replies arrive after a caller has given up. They must be dropped
    // quietly, not panic the subscriber task and take every other waiter with it.
    let dir = tempfile::tempdir().unwrap();
    let (ecfg, lcfg) = pair(dir.path());
    let mut engine = Runner::boot(ecfg.clone()).await.unwrap();
    let lb = Loopback::connect(lcfg).await.unwrap();

    // Publish a reply nobody is waiting for.
    let client = redis::Client::open(ecfg.redis_url.as_str()).unwrap();
    let mut c = client.get_multiplexed_async_connection().await.unwrap();
    let orphan = cex_proto::Response::ok(Uuid::new_v4(), ResponseBody::Ack);
    let _: i64 = redis::AsyncCommands::publish(
        &mut c,
        &ecfg.responses_channel,
        serde_json::to_string(&orphan).unwrap(),
    )
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // The loopback must still work afterwards.
    let alice = Uuid::new_v4();
    let sent = tokio::spawn({
        let lb = lb.clone();
        async move { lb.command(deposit(alice, "USDT", 42)).await }
    });
    pump(&mut engine).await;

    assert!(
        sent.await.unwrap().is_ok(),
        "subscriber died on an orphan reply"
    );
}

// ───────────────────────── request ids ─────────────────────────

#[tokio::test]
async fn the_caller_does_not_have_to_supply_a_unique_request_id() {
    // Two commands built with the same id must not collide: the loopback stamps
    // its own, so a careless caller cannot cross two requests.
    let dir = tempfile::tempdir().unwrap();
    let (ecfg, lcfg) = pair(dir.path());
    let engine = Runner::boot(ecfg).await.unwrap();
    let handle = spawn_engine(engine);
    let lb = Loopback::connect(lcfg).await.unwrap();

    let fixed = Uuid::from_u128(999);
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();

    let mut first = deposit(a, "USDT", 10);
    let mut second = deposit(b, "USDT", 20);
    if let Command::Deposit { request_id, .. } = &mut first {
        *request_id = fixed;
    }
    if let Command::Deposit { request_id, .. } = &mut second {
        *request_id = fixed;
    }

    assert!(lb.command(first).await.is_ok());
    assert!(lb.command(second).await.is_ok());

    // Both landed, on the right accounts.
    for (who, amount) in [(a, 10i64), (b, 20i64)] {
        let body = lb
            .query(Query::Balances {
                request_id: fixed,
                user_id: who,
            })
            .await
            .unwrap();
        match body {
            ResponseBody::Balances(v) => assert_eq!(v[0].available, amount),
            other => panic!("expected Balances, got {other:?}"),
        }
    }
    handle.abort();
}
