//! The persister loop, against a real Redis and a real Postgres.
//!
//! These need the compose stack up (`docker compose up -d`). Each test gets its
//! own stream, consumer group and Postgres schema.
//!
//! The one that matters most is
//! `an_engine_running_ahead_while_the_persister_is_down_loses_nothing`: it is
//! the whole reason this is a separate process. The engine must never wait on a
//! database, so the persister falling behind — or being down entirely — has to
//! cost history freshness and nothing else.

use cex_engine::config::Config as EngineConfig;
use cex_engine::runner::Runner;
use cex_persist::{Config, Consumer, HistoryStore};
use cex_proto::{
    Command, Event, EventBatch, OrderType, Side, TimeInForce, UserId, FIELD_PAYLOAD,
};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

fn database_url() -> String {
    std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into())
}

/// A persister pointed at throwaway infrastructure. `tag` ties the Redis stream
/// and the Postgres schema together so one test can restart its consumer
/// against the same state.
fn test_config(tag: &str) -> Config {
    Config {
        redis_url: redis_url(),
        database_url: database_url(),
        schema: format!("t{tag}"),
        events_stream: format!("test:{tag}:events"),
        group: format!("test:{tag}:group"),
        consumer: "persist-1".into(),
        count: 256,
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

async fn store_for(cfg: &Config) -> HistoryStore {
    HistoryStore::connect_to_schema(&cfg.database_url, &cfg.schema)
        .await
        .expect("postgres — is `docker compose up -d` running?")
}

async fn boot(cfg: &Config) -> Consumer {
    Consumer::boot(cfg.clone(), store_for(cfg).await)
        .await
        .expect("consumer boot")
}

/// Put a batch on the events stream exactly as the engine does.
async fn publish(conn: &mut redis::aio::MultiplexedConnection, cfg: &Config, batch: &EventBatch) {
    let json = serde_json::to_string(batch).unwrap();
    let _: String = conn
        .xadd(&cfg.events_stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
        .await
        .expect("xadd");
}

/// Read the whole stream until it goes quiet.
async fn drain(c: &mut Consumer) -> usize {
    let mut total = 0;
    loop {
        let n = c.step().await.expect("step");
        if n == 0 {
            return total;
        }
        total += n;
    }
}

fn batch(seq: u64, events: Vec<Event>) -> EventBatch {
    EventBatch {
        seq,
        request_id: Uuid::new_v4(),
        events,
    }
}

fn accepted(order_id: u64, user: UserId, qty: i64) -> Event {
    Event::OrderAccepted {
        order_id,
        user_id: user,
        symbol: SYM.into(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        price: Some(P50K),
        qty,
    }
}

// ───────────────────────── the happy path ─────────────────────────

#[tokio::test]
async fn a_published_batch_reaches_postgres() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut c = boot(&cfg).await;
    let alice = Uuid::new_v4();

    publish(&mut r, &cfg, &batch(1, vec![accepted(1, alice, Q1)])).await;

    assert_eq!(drain(&mut c).await, 1);
    assert_eq!(c.store().written_seqs().await.unwrap(), vec![1]);
    assert_eq!(c.store().order(1).await.unwrap().unwrap().qty, Q1);
}

#[tokio::test]
async fn batches_already_on_the_stream_before_boot_are_not_skipped() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let alice = Uuid::new_v4();

    // The group has to be created at the head of the stream, not at its tail,
    // or a persister deployed after the exchange started would silently begin
    // its history partway through.
    publish(&mut r, &cfg, &batch(1, vec![accepted(1, alice, Q1)])).await;
    publish(&mut r, &cfg, &batch(2, vec![accepted(2, alice, Q1)])).await;

    let mut c = boot(&cfg).await;
    drain(&mut c).await;

    assert_eq!(c.store().written_seqs().await.unwrap(), vec![1, 2]);
}

// ───────────────────────── redelivery ─────────────────────────

#[tokio::test]
async fn a_republished_batch_does_not_duplicate_rows() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut c = boot(&cfg).await;
    let maker = Uuid::new_v4();
    let taker = Uuid::new_v4();

    let traded = batch(
        1,
        vec![Event::Trades {
            symbol: SYM.into(),
            fills: vec![cex_proto::Fill {
                symbol: SYM.into(),
                price: P50K,
                qty: Q1,
                maker_order_id: 1,
                taker_order_id: 2,
                maker_user_id: maker,
                taker_user_id: taker,
                taker_side: Side::Buy,
                notional: 50_000_000,
                maker_fee: 500,
                taker_fee: 100,
            }],
        }],
    );

    publish(&mut r, &cfg, &traded).await;
    drain(&mut c).await;

    // Recovery: the engine replays the command log and publishes the same seq
    // again under a new stream id. This is not a hypothetical, it is what the
    // engine does on every restart.
    publish(&mut r, &cfg, &traded).await;
    let handled = drain(&mut c).await;

    assert_eq!(handled, 1, "the entry was read and acknowledged");
    assert_eq!(
        c.store().fills_for_symbol(SYM, 100).await.unwrap().len(),
        1,
        "but the trade was only recorded once"
    );
}

// ───────────────────────── crash and restart ─────────────────────────

/// Claim entries into the group without acknowledging them — the state a
/// consumer leaves behind when it is killed after delivery and before its
/// write commits.
async fn claim_without_acking(cfg: &Config, r: &mut redis::aio::MultiplexedConnection) -> usize {
    let opts = StreamReadOptions::default()
        .group(&cfg.group, &cfg.consumer)
        .count(256)
        .block(100);
    let reply: Option<StreamReadReply> = r
        .xread_options(&[&cfg.events_stream], &[">"], &opts)
        .await
        .expect("xreadgroup");
    reply
        .map(|rep| rep.keys.iter().map(|k| k.ids.len()).sum())
        .unwrap_or(0)
}

#[tokio::test]
async fn a_restarted_consumer_picks_up_what_it_never_acknowledged() {
    let tag = Uuid::new_v4().simple().to_string();
    let cfg = test_config(&tag);
    let mut r = conn(&cfg).await;
    let alice = Uuid::new_v4();

    // Boot once so the group exists, then abandon that consumer.
    let _ = boot(&cfg).await;

    publish(&mut r, &cfg, &batch(1, vec![accepted(1, alice, Q1)])).await;
    publish(&mut r, &cfg, &batch(2, vec![accepted(2, alice, Q1)])).await;
    publish(&mut r, &cfg, &batch(3, vec![accepted(3, alice, Q1)])).await;

    // Delivered to `persist-1`, never acknowledged: the process died holding them.
    assert_eq!(claim_without_acking(&cfg, &mut r).await, 3);

    // A fresh consumer under the same name must find its own backlog. Reading
    // only new entries (`>`) would step straight past all three.
    let mut restarted = boot(&cfg).await;
    drain(&mut restarted).await;

    assert_eq!(
        restarted.store().written_seqs().await.unwrap(),
        vec![1, 2, 3],
        "nothing was lost across the restart"
    );
}

#[tokio::test]
async fn a_restart_after_a_committed_write_does_not_duplicate_it() {
    let tag = Uuid::new_v4().simple().to_string();
    let cfg = test_config(&tag);
    let mut r = conn(&cfg).await;
    let alice = Uuid::new_v4();

    let _ = boot(&cfg).await;
    publish(&mut r, &cfg, &batch(1, vec![accepted(1, alice, Q1)])).await;

    // The gap that has to be safe: the transaction committed, and the process
    // died before the acknowledgement reached Redis. Redis will hand the entry
    // back, and the write must not happen twice.
    let store = store_for(&cfg).await;
    store
        .write_batches(&[batch(1, vec![accepted(1, alice, Q1)])])
        .await
        .unwrap();
    assert_eq!(claim_without_acking(&cfg, &mut r).await, 1);

    let mut restarted = boot(&cfg).await;
    drain(&mut restarted).await;

    assert_eq!(restarted.store().written_seqs().await.unwrap(), vec![1]);
}

#[tokio::test]
async fn a_consumer_resumes_where_it_left_off_rather_than_from_the_start() {
    let tag = Uuid::new_v4().simple().to_string();
    let cfg = test_config(&tag);
    let mut r = conn(&cfg).await;
    let alice = Uuid::new_v4();

    let mut first = boot(&cfg).await;
    publish(&mut r, &cfg, &batch(1, vec![accepted(1, alice, Q1)])).await;
    assert_eq!(drain(&mut first).await, 1);
    drop(first);

    publish(&mut r, &cfg, &batch(2, vec![accepted(2, alice, Q1)])).await;

    let mut second = boot(&cfg).await;
    let handled = drain(&mut second).await;

    assert_eq!(handled, 1, "only the entry published after the restart");
    assert_eq!(second.store().written_seqs().await.unwrap(), vec![1, 2]);
}

// ───────────────────────── bad input ─────────────────────────

#[tokio::test]
async fn an_undecodable_entry_is_acknowledged_and_does_not_wedge_the_stream() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut c = boot(&cfg).await;
    let alice = Uuid::new_v4();

    // Nothing can ever be written for this, so holding it unacknowledged would
    // block the group forever and stop all history behind it.
    let _: String = r
        .xadd(
            &cfg.events_stream,
            "*",
            &[(FIELD_PAYLOAD, "{not json at all")],
        )
        .await
        .unwrap();
    publish(&mut r, &cfg, &batch(1, vec![accepted(1, alice, Q1)])).await;

    drain(&mut c).await;
    assert_eq!(
        c.store().written_seqs().await.unwrap(),
        vec![1],
        "the good batch behind the bad entry still landed"
    );

    // And the bad entry is gone from the group's pending list, so a restart
    // does not trip over it again.
    let mut restarted = boot(&cfg).await;
    assert_eq!(drain(&mut restarted).await, 0);
}

// ───────────────────────── the engine never waits ─────────────────────────

#[tokio::test]
async fn an_engine_running_ahead_while_the_persister_is_down_loses_nothing() {
    let tag = Uuid::new_v4().simple().to_string();
    let cfg = test_config(&tag);
    let dir = tempfile::tempdir().unwrap();

    let engine_cfg = EngineConfig {
        redis_url: redis_url(),
        commands_stream: format!("test:{tag}:commands"),
        events_stream: cfg.events_stream.clone(),
        responses_channel: format!("test:{tag}:responses"),
        queries_queue: format!("test:{tag}:queries"),
        snapshot_dir: dir.path().to_path_buf(),
        snapshot_every: 1_000_000,
        snapshot_keep: 3,
        block_ms: 150,
        lock_ttl_ms: 30_000,
    };

    // The group must exist before the engine publishes, or Redis has nowhere to
    // hold the backlog. Booting the persister and then dropping it is exactly
    // the "persister is down" case.
    let _ = boot(&cfg).await;

    let r = conn(&cfg).await;
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    let send = |cmd: &Command| {
        let json = serde_json::to_string(cmd).unwrap();
        let stream = engine_cfg.commands_stream.clone();
        let mut r = r.clone();
        async move {
            let _: String = r
                .xadd(&stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
                .await
                .expect("xadd");
        }
    };

    send(&Command::Deposit {
        request_id: Uuid::new_v4(),
        user_id: alice,
        asset: "BTC".into(),
        amount: 100_000_000,
    })
    .await;
    send(&Command::Deposit {
        request_id: Uuid::new_v4(),
        user_id: bob,
        asset: "USDT".into(),
        amount: 10_000_000_000_000,
    })
    .await;

    // 60 resting sells, then 60 buys that lift them. Plenty of history piling
    // up on a stream nobody is reading.
    const N: usize = 60;
    for _ in 0..N {
        send(&Command::PlaceOrder {
            request_id: Uuid::new_v4(),
            user_id: alice,
            symbol: SYM.into(),
            side: Side::Sell,
            order_type: OrderType::Limit,
            time_in_force: Some(TimeInForce::Gtc),
            price: Some(P50K),
            qty: Q1,
        })
        .await;
    }
    for _ in 0..N {
        send(&Command::PlaceOrder {
            request_id: Uuid::new_v4(),
            user_id: bob,
            symbol: SYM.into(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            time_in_force: Some(TimeInForce::Gtc),
            price: Some(P50K),
            qty: Q1,
        })
        .await;
    }

    let mut runner = Runner::boot(engine_cfg).await.expect("engine boot");
    let mut applied = 0;
    loop {
        let n = runner.step().await.expect("step");
        if n == 0 {
            break;
        }
        applied += n;
    }
    assert_eq!(
        applied,
        2 + N * 2,
        "the engine applied everything with no persister running at all"
    );

    // Now let history catch up. Nothing was lost while it was down.
    let mut c = boot(&cfg).await;
    drain(&mut c).await;

    let seqs = c.store().written_seqs().await.unwrap();
    assert_eq!(
        seqs.len(),
        2 + N * 2,
        "every batch the engine published was written"
    );
    assert_eq!(
        seqs,
        (1..=(2 + N * 2) as u64).collect::<Vec<_>>(),
        "gap-free and in order"
    );

    let fills = c.store().fills_for_symbol(SYM, 1_000).await.unwrap();
    assert_eq!(fills.len(), N, "every trade the engine printed was recorded");
    assert!(fills.iter().all(|f| f.price == P50K));
    assert!(fills.iter().all(|f| f.maker_user_id == alice));
    assert!(fills.iter().all(|f| f.taker_user_id == bob));
}
