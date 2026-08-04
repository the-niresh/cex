//! End-to-end tests against a real Redis.
//!
//! These need the compose stack up (`docker compose up -d`). Each test uses its
//! own randomly-named streams, so they neither collide with each other nor with
//! a running engine.
//!
//! The test that matters is `a_crash_mid_stream_loses_nothing`: apply a load of
//! commands, drop the engine on the floor without warning, start a fresh one,
//! and prove it ends up in exactly the state it would have reached had nothing
//! happened.

use cex_engine::config::Config;
use cex_engine::runner::Runner;
use cex_proto::{
    Command, EventBatch, OrderType, Query, ResponseBody, Side, TimeInForce, FIELD_PAYLOAD,
};
use redis::AsyncCommands;
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

/// Config pointing at throwaway streams and a throwaway snapshot directory.
fn test_config(dir: &std::path::Path) -> Config {
    let tag = Uuid::new_v4().simple().to_string();
    Config {
        redis_url: redis_url(),
        commands_stream: format!("test:{tag}:commands"),
        events_stream: format!("test:{tag}:events"),
        responses_channel: format!("test:{tag}:responses"),
        queries_queue: format!("test:{tag}:queries"),
        snapshot_dir: dir.to_path_buf(),
        snapshot_every: 1_000_000, // off unless a test asks for it
        snapshot_keep: 3,
        // Short, so a drained stream returns promptly instead of hanging the test.
        block_ms: 150,
    }
}

async fn conn(cfg: &Config) -> redis::aio::MultiplexedConnection {
    redis::Client::open(cfg.redis_url.as_str())
        .expect("redis client")
        .get_multiplexed_async_connection()
        .await
        .expect("redis connection — is `docker compose up -d` running?")
}

async fn send(conn: &mut redis::aio::MultiplexedConnection, cfg: &Config, cmd: &Command) {
    let json = serde_json::to_string(cmd).unwrap();
    let _: String = conn
        .xadd(&cfg.commands_stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
        .await
        .expect("xadd");
}

/// Drain the command stream, applying everything on it.
async fn drain(runner: &mut Runner) -> usize {
    let mut total = 0;
    loop {
        let n = runner.step().await.expect("step");
        if n == 0 {
            return total;
        }
        total += n;
    }
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

fn balances(runner: &Runner, who: Uuid) -> Vec<(String, i64, i64)> {
    match runner
        .state()
        .query(&Query::Balances {
            request_id: Uuid::nil(),
            user_id: who,
        })
        .unwrap()
    {
        ResponseBody::Balances(v) => v
            .into_iter()
            .map(|b| (b.asset, b.available, b.locked))
            .collect(),
        other => panic!("expected Balances, got {other:?}"),
    }
}

fn depth(runner: &Runner) -> (Vec<[i64; 2]>, Vec<[i64; 2]>) {
    match runner
        .state()
        .query(&Query::Depth {
            request_id: Uuid::nil(),
            symbol: SYM.into(),
            limit: None,
        })
        .unwrap()
    {
        ResponseBody::Depth(d) => (d.bids, d.asks),
        other => panic!("expected Depth, got {other:?}"),
    }
}

// ───────────────────────── basic consumption ─────────────────────────

#[tokio::test]
async fn commands_on_the_stream_are_applied_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let alice = Uuid::new_v4();

    send(&mut c, &cfg, &deposit(alice, "USDT", 1_000_000_000)).await;
    send(&mut c, &cfg, &limit(alice, Side::Buy, P50K, Q1)).await;

    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    assert_eq!(drain(&mut runner).await, 2);

    let usdt = balances(&runner, alice);
    assert_eq!(usdt, vec![("USDT".into(), 950_000_000, 50_000_000)]);
    assert_eq!(depth(&runner).0, vec![[P50K, Q1]]);
    runner.state().check_invariants().unwrap();
}

#[tokio::test]
async fn an_idle_stream_applies_nothing_and_returns_promptly() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut runner = Runner::boot(cfg).await.unwrap();

    assert_eq!(runner.step().await.unwrap(), 0);
}

#[tokio::test]
async fn a_malformed_command_is_skipped_without_stopping_the_loop() {
    // One bad message must never wedge the exchange.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let alice = Uuid::new_v4();

    let _: String = c
        .xadd(&cfg.commands_stream, "*", &[(FIELD_PAYLOAD, "{ not json")])
        .await
        .unwrap();
    send(&mut c, &cfg, &deposit(alice, "USDT", 500)).await;

    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain(&mut runner).await;

    assert_eq!(
        balances(&runner, alice),
        vec![("USDT".into(), 500, 0)],
        "the good command after the bad one must still apply"
    );
}

#[tokio::test]
async fn a_rejected_command_does_not_change_state() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let poor = Uuid::new_v4();

    // No funds, so this is rejected by the engine rather than by the transport.
    send(&mut c, &cfg, &limit(poor, Side::Buy, P50K, Q1)).await;

    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain(&mut runner).await;

    assert!(balances(&runner, poor).is_empty());
    assert!(depth(&runner).0.is_empty());
}

// ───────────────────────── events ─────────────────────────

#[tokio::test]
async fn a_trade_publishes_an_event_batch_carrying_the_fill() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    send(&mut c, &cfg, &deposit(alice, "USDT", 1_000_000_000)).await;
    send(&mut c, &cfg, &deposit(bob, "BTC", 100_000_000)).await;
    send(&mut c, &cfg, &limit(bob, Side::Sell, P50K, Q1)).await;
    send(&mut c, &cfg, &limit(alice, Side::Buy, P50K, Q1)).await;

    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain(&mut runner).await;

    // Read everything the engine published.
    let entries: Vec<(String, Vec<(String, String)>)> = redis::cmd("XRANGE")
        .arg(&cfg.events_stream)
        .arg("-")
        .arg("+")
        .query_async(&mut c)
        .await
        .unwrap();
    assert!(!entries.is_empty(), "no events were published");

    let batches: Vec<EventBatch> = entries
        .iter()
        .map(|(_, fields)| {
            let payload = &fields.iter().find(|(k, _)| k == FIELD_PAYLOAD).unwrap().1;
            serde_json::from_str(payload).unwrap()
        })
        .collect();

    let fills: Vec<_> = batches
        .iter()
        .flat_map(|b| &b.events)
        .filter_map(|e| match e {
            cex_proto::Event::Trades { fills, .. } => Some(fills.clone()),
            _ => None,
        })
        .flatten()
        .collect();

    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].price, P50K);
    assert_eq!(fills[0].maker_user_id, bob);
    assert_eq!(fills[0].taker_user_id, alice);

    // Sequence numbers must be gap-free and ascending across the whole run.
    let seqs: Vec<u64> = batches.iter().map(|b| b.seq).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "event batches published out of order");
}

// ───────────────────────── recovery ─────────────────────────

#[tokio::test]
async fn a_snapshot_records_the_position_it_was_taken_at() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let alice = Uuid::new_v4();

    send(&mut c, &cfg, &deposit(alice, "USDT", 1_000)).await;

    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain(&mut runner).await;
    let position = runner.position();
    runner.snapshot().await.unwrap();

    let store = runner.store();
    assert_eq!(store.resume_position().unwrap(), position);
}

#[tokio::test]
async fn a_crash_mid_stream_loses_nothing() {
    // The gate for this phase.
    //
    // Send a long run of commands. Let one engine consume part of it and
    // snapshot. Throw that engine away without a clean shutdown, boot a fresh
    // one against the same snapshot directory, and let it finish. It must land
    // in exactly the state a single uninterrupted engine would have reached.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;

    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    let mut script: Vec<Command> = vec![
        deposit(alice, "USDT", 100_000_000_000),
        deposit(bob, "BTC", 10_000_000_000),
        deposit(bob, "USDT", 100_000_000_000),
    ];
    // Crossing bands, so this actually trades rather than just stacking orders.
    for i in 0..600i64 {
        let price = P50K + (i % 9) * 10_000_000;
        let price = price - price % 10_000;
        if i % 2 == 0 {
            script.push(limit(alice, Side::Buy, price, Q1));
        } else {
            script.push(limit(bob, Side::Sell, price, Q1));
        }
    }
    for cmd in &script {
        send(&mut c, &cfg, cmd).await;
    }

    // A control engine that never restarts.
    let control_dir = tempfile::tempdir().unwrap();
    let mut control_cfg = cfg.clone();
    control_cfg.snapshot_dir = control_dir.path().to_path_buf();
    let mut control = Runner::boot(control_cfg).await.unwrap();
    drain(&mut control).await;

    // The engine that dies. Consume a couple of batches, snapshot, then drop it.
    let mut victim = Runner::boot(cfg.clone()).await.unwrap();
    let mut consumed = 0;
    for _ in 0..2 {
        consumed += victim.step().await.unwrap();
    }
    assert!(consumed > 0, "the victim should have applied something");
    assert!(
        consumed < script.len(),
        "the victim must not have finished, or nothing is being tested"
    );
    victim.snapshot().await.unwrap();
    let snapshot_position = victim.position();
    drop(victim); // no clean shutdown, no final flush

    // A brand new process boots from the snapshot directory alone.
    let mut recovered = Runner::boot(cfg.clone()).await.unwrap();
    assert_eq!(
        recovered.position(),
        snapshot_position,
        "did not resume from the snapshot"
    );
    drain(&mut recovered).await;

    assert_eq!(recovered.state().seq(), control.state().seq(), "seq diverged");
    assert_eq!(balances(&recovered, alice), balances(&control, alice));
    assert_eq!(balances(&recovered, bob), balances(&control, bob));
    assert_eq!(depth(&recovered), depth(&control));
    assert_eq!(
        recovered.state().open_order_ids(),
        control.state().open_order_ids()
    );
    recovered.state().check_invariants().unwrap();
}

#[tokio::test]
async fn booting_with_no_snapshot_replays_the_whole_log() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let alice = Uuid::new_v4();

    send(&mut c, &cfg, &deposit(alice, "USDT", 700)).await;
    send(&mut c, &cfg, &deposit(alice, "USDT", 300)).await;

    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    assert_eq!(runner.position(), cex_engine::StreamId::ZERO);
    drain(&mut runner).await;

    assert_eq!(balances(&runner, alice), vec![("USDT".into(), 1_000, 0)]);
}

#[tokio::test]
async fn a_replayed_command_is_not_applied_twice() {
    // Booting from a snapshot must resume *after* the recorded id, not at it.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let alice = Uuid::new_v4();

    send(&mut c, &cfg, &deposit(alice, "USDT", 1_000)).await;

    let mut first = Runner::boot(cfg.clone()).await.unwrap();
    drain(&mut first).await;
    first.snapshot().await.unwrap();
    assert_eq!(balances(&first, alice), vec![("USDT".into(), 1_000, 0)]);
    drop(first);

    let mut second = Runner::boot(cfg.clone()).await.unwrap();
    drain(&mut second).await;

    assert_eq!(
        balances(&second, alice),
        vec![("USDT".into(), 1_000, 0)],
        "the deposit was applied a second time"
    );
}

#[tokio::test]
async fn snapshots_are_taken_automatically_on_the_configured_interval() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = test_config(dir.path());
    cfg.snapshot_every = 3;
    let mut c = conn(&cfg).await;
    let alice = Uuid::new_v4();

    for _ in 0..7 {
        send(&mut c, &cfg, &deposit(alice, "USDT", 10)).await;
    }

    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain(&mut runner).await;

    assert!(
        !runner.store().list().unwrap().is_empty(),
        "no snapshot was written despite passing the interval"
    );
}
