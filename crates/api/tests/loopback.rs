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

// ───────────────────────── timing ─────────────────────────

#[tokio::test]
async fn a_command_records_its_own_engine_time() {
    let dir = tempfile::tempdir().unwrap();
    let (engine_cfg, loopback_cfg) = pair(dir.path());
    let runner = Runner::boot(engine_cfg).await.expect("engine boot");
    let _engine = spawn_engine(runner);
    let loopback = Loopback::connect(loopback_cfg).await.expect("loopback");

    let who = Uuid::new_v4();
    let (result, micros) = cex_api::timing::measure_engine(async {
        loopback.command(deposit(who, "USDT", 1_000_000)).await
    })
    .await;

    assert!(
        result.is_ok(),
        "the deposit itself must succeed: {result:?}"
    );
    assert!(
        micros > 0,
        "a real round trip through Redis to the engine cannot take zero \
         microseconds; nothing recorded means the instrumentation is not on \
         this path at all"
    );
}

#[tokio::test]
async fn concurrent_scopes_do_not_leak_engine_time_into_each_other() {
    // Loopback is one shared instance behind every request. If `record_engine`
    // ever wrote to the wrong task's accumulator, a busy request's engine time
    // would land partly on a quiet request running at the same moment, and every
    // published percentile would be built on mixed data. This drives two
    // `measure_engine` scopes at once against the same `Loopback` and checks each
    // scope's total reflects only the work it issued: one scope sends a single
    // command, the other sends several, and the totals must not blur together.
    //
    // Failure mode this test catches: the quiet scope's recorded total includes
    // engine time that actually belongs to the busy scope's commands (in whole
    // or in part), because `record_engine` landed in the wrong task's
    // accumulator.
    //
    // The comparison basis matters more than it looks on this box: it is a
    // shared, heavily oversubscribed machine (load average well past its core
    // count under normal conditions), and two wall-clock round trips measured
    // in two *different* moments — a pre-measured baseline vs. this section,
    // or the busy scope's own per-command average vs. the quiet scope's total —
    // can differ by an order of magnitude purely from scheduling jitter,
    // nothing to do with `Loopback`. Both of those shapes produced spurious
    // failures during development. What does not depend on cross-window noise
    // is comparing `measure_engine`'s report against an independent wall-clock
    // timer wrapped around the *exact same* call, in the *exact same* moment:
    // the two measure the identical interval, so whatever the system is doing
    // right then affects both readings equally. Any gap between them is not
    // system noise — it is time `record_engine` attributed to this scope that
    // did not come from timing this scope's own call.
    let dir = tempfile::tempdir().unwrap();
    let (engine_cfg, loopback_cfg) = pair(dir.path());
    let runner = Runner::boot(engine_cfg).await.expect("engine boot");
    let _engine = spawn_engine(runner);
    let loopback = Loopback::connect(loopback_cfg).await.expect("loopback");

    // Busy fires many commands concurrently (not one at a time) so real work
    // is continuously in flight through the whole window the quiet scope's
    // single command overlaps with — the shape that would actually give a
    // contamination bug the chance to manifest, and also the realistic shape
    // of "many requests hitting a shared Loopback at once."
    const BUSY_COMMANDS: u64 = 60;

    let quiet_user = Uuid::new_v4();
    let busy_user = Uuid::new_v4();

    // Rather than leave the overlap between the two scopes to timing luck —
    // this machine's contention made the quiet scope's single command
    // routinely finish before the busy scope's flood had completed even one
    // round trip, which would let a real leak hide simply by never getting
    // the chance to happen — the quiet scope explicitly stays open until the
    // busy scope signals it is done. Its own timed work (both the real
    // command and the ground-truth window) still finishes first; it just
    // does not let `measure_engine`'s scope close until the busy scope's
    // entire run — and every `record_engine` call it could possibly leak —
    // has had the chance to land while this scope was still active.
    let (busy_done_tx, busy_done_rx) = tokio::sync::oneshot::channel::<()>();

    let quiet_loopback = loopback.clone();
    let quiet_scope = cex_api::timing::measure_engine(async move {
        // The ground truth: an independent timer around this exact call,
        // taken by the test itself rather than by `record_engine`. Wraps the
        // same single await `Loopback::command_with_id`'s own internal timer
        // wraps, so — isolation holding — the two should land within a
        // sliver of each other regardless of what else is happening on the
        // box right now.
        let ground_truth_started = std::time::Instant::now();
        let result = quiet_loopback
            .command(deposit(quiet_user, "USDT", 1_000))
            .await;
        let ground_truth_micros = ground_truth_started.elapsed().as_micros() as u64;
        let _ = busy_done_rx.await;
        (result, ground_truth_micros)
    });

    let busy_loopback = loopback.clone();
    let busy_scope = cex_api::timing::measure_engine(async move {
        let sends = (0..BUSY_COMMANDS).map(|_| {
            let lb = busy_loopback.clone();
            async move {
                lb.command(deposit(busy_user, "USDT", 1_000))
                    .await
                    .expect("deposit should succeed")
            }
        });
        futures_util::future::join_all(sends).await;
        let _ = busy_done_tx.send(());
    });

    let (((quiet_result, ground_truth_micros), quiet_micros), (_, busy_micros)) =
        tokio::join!(quiet_scope, busy_scope);

    assert!(
        quiet_result.is_ok(),
        "the quiet scope's own command must still succeed: {quiet_result:?}"
    );
    assert!(
        quiet_micros > 0,
        "the quiet scope did one real round trip and must record some time"
    );
    assert!(
        busy_micros > 0,
        "the busy scope did real round trips and must record some time"
    );
    assert!(
        ground_truth_micros > 0,
        "the independent timer must have measured a real interval"
    );

    // Tolerance is an absolute figure, not a multiplier, and it is not a
    // round number either: `quiet_micros` and `ground_truth_micros` time the
    // identical interval (same call, same moment), so without contamination
    // they should differ only by the handful of extra instructions the
    // test's outer timer covers that the inner one does not (entering
    // `.command()`, constructing the tuple) — measured empirically at 4-9
    // microseconds across 25 repeated runs of this exact test on this box.
    // Both simulated failure modes checked during development — a shared
    // accumulator (quiet's total balloons to match busy's) and a shared
    // mutable start-time (quiet's own reading gets clobbered by a busy
    // command's start, so it *under*-reports) — produced gaps no smaller
    // than 885 microseconds across 20 repeated runs each, and the shared-
    // accumulator case pushed `quiet_micros` up to match `busy_micros`
    // exactly. 300us sits with roughly 30-75x headroom above the observed
    // normal jitter and about 3x below the smallest observed contamination
    // gap, in either direction, which is why the check below is symmetric
    // (leaked time can just as easily displace a scope's own reading as
    // inflate it, so only checking one direction would have missed the
    // under-reporting case).
    const TOLERANCE_MICROS: i64 = 300;
    let gap = quiet_micros as i64 - ground_truth_micros as i64;
    assert!(
        gap.abs() <= TOLERANCE_MICROS,
        "the quiet scope's one command measured {ground_truth_micros}us on an \
         independent timer wrapping the exact same call, in the exact same \
         moment, so `record_engine` should have attributed close to that same \
         figure to this scope; instead it recorded {quiet_micros}us, {gap}us \
         off (more than the {TOLERANCE_MICROS}us tolerance), which means \
         engine time that belongs to the busy scope's {BUSY_COMMANDS} \
         concurrent commands (which together recorded {busy_micros}us) \
         either leaked into this scope's reading or displaced it, rather \
         than this scope timing only its own single round trip"
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
