//! The event stream reader, against a real Redis.
//!
//! Needs the compose stack up (`docker compose up -d`). Each test gets its own
//! stream and consumer group.
//!
//! The behaviour worth pinning down is the one that differs from the
//! persister's: this group starts at the *tail*, because replaying stale depth
//! deltas into a live feed is worse than not having them.

use cex_proto::{Event, EventBatch, OrderType, Side, FIELD_PAYLOAD};
use cex_ws::wire::Channel;
use cex_ws::{Config, Feed};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6390".into())
}

fn test_config(tag: &str) -> Config {
    Config {
        redis_url: redis_url(),
        events_stream: format!("test:{tag}:events"),
        group: format!("test:{tag}:group"),
        consumer: "ws-1".into(),
        bind: "127.0.0.1:0".into(),
        broadcast_capacity: 256,
        count: 256,
        // Short, so a quiet stream returns promptly instead of hanging the test.
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

async fn publish(conn: &mut redis::aio::MultiplexedConnection, cfg: &Config, batch: &EventBatch) {
    let json = serde_json::to_string(batch).unwrap();
    let _: String = conn
        .xadd(&cfg.events_stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
        .await
        .expect("xadd");
}

async fn drain(f: &mut Feed) -> usize {
    let mut total = 0;
    loop {
        let n = f.step().await.expect("step");
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

fn accepted(order_id: u64, user: Uuid) -> Event {
    Event::OrderAccepted {
        order_id,
        user_id: user,
        symbol: SYM.into(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        price: Some(P50K),
        qty: Q1,
    }
}

fn depth(depth_seq: u64) -> Event {
    Event::DepthUpdated {
        symbol: SYM.into(),
        depth_seq,
        deltas: vec![],
    }
}

// ───────────────────────── the happy path ─────────────────────────

#[tokio::test]
async fn a_published_batch_reaches_subscribers() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut rx = f.subscribe();

    publish(&mut r, &cfg, &batch(1, vec![depth(4)])).await;
    assert_eq!(drain(&mut f).await, 1);

    let update = rx.try_recv().expect("an update");
    assert_eq!(update.channel, Channel::Depth(SYM.into()));
    assert_eq!(update.seq, 1);
}

#[tokio::test]
async fn every_subscriber_gets_its_own_copy() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut a = f.subscribe();
    let mut b = f.subscribe();

    publish(&mut r, &cfg, &batch(1, vec![depth(4)])).await;
    drain(&mut f).await;

    // The stream is read once; the fan-out is what multiplies it.
    assert_eq!(a.try_recv().expect("a").seq, 1);
    assert_eq!(b.try_recv().expect("b").seq, 1);
}

#[tokio::test]
async fn one_batch_can_produce_several_addressed_updates() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut rx = f.subscribe();
    let alice = Uuid::new_v4();

    publish(&mut r, &cfg, &batch(1, vec![accepted(1, alice), depth(4)])).await;
    drain(&mut f).await;

    let first = rx.try_recv().expect("the order update");
    assert_eq!(first.channel, Channel::Orders);
    assert_eq!(first.audience, Some(alice));

    let second = rx.try_recv().expect("the depth update");
    assert_eq!(second.channel, Channel::Depth(SYM.into()));
    assert_eq!(second.audience, None);
}

// ───────────────────────── live, not historical ─────────────────────────

#[tokio::test]
async fn entries_published_before_boot_are_not_replayed() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;

    // Market data has a shelf life. Unlike the persister, this service must not
    // start by replaying a backlog — a client would rebuild a book from deltas
    // that stopped being true hours ago.
    publish(&mut r, &cfg, &batch(1, vec![depth(1)])).await;
    publish(&mut r, &cfg, &batch(2, vec![depth(2)])).await;

    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut rx = f.subscribe();
    assert_eq!(drain(&mut f).await, 0, "history was replayed");
    assert!(rx.try_recv().is_err());

    // But anything published from now on does arrive.
    publish(&mut r, &cfg, &batch(3, vec![depth(3)])).await;
    assert_eq!(drain(&mut f).await, 1);
    assert_eq!(rx.try_recv().expect("the live update").seq, 3);
}

#[tokio::test]
async fn stale_pending_entries_are_cleared_without_being_broadcast() {
    let tag = Uuid::new_v4().simple().to_string();
    let cfg = test_config(&tag);
    let mut r = conn(&cfg).await;

    // Boot once so the group exists, then leave entries delivered and unacked —
    // what a killed instance leaves behind.
    let _ = Feed::boot(cfg.clone()).await.expect("feed boot");
    publish(&mut r, &cfg, &batch(1, vec![depth(1)])).await;
    publish(&mut r, &cfg, &batch(2, vec![depth(2)])).await;

    let opts = StreamReadOptions::default()
        .group(&cfg.group, &cfg.consumer)
        .count(256)
        .block(100);
    let claimed: Option<StreamReadReply> = r
        .xread_options(&[&cfg.events_stream], &[">"], &opts)
        .await
        .unwrap();
    let pending: usize = claimed
        .map(|rep| rep.keys.iter().map(|k| k.ids.len()).sum())
        .unwrap_or(0);
    assert_eq!(pending, 2);

    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut rx = f.subscribe();
    drain(&mut f).await;

    assert!(
        rx.try_recv().is_err(),
        "stale entries were broadcast as if they were live"
    );

    // But they are acknowledged, so the pending list does not grow forever.
    let still_pending: usize = redis::cmd("XPENDING")
        .arg(&cfg.events_stream)
        .arg(&cfg.group)
        .query_async(&mut r)
        .await
        .map(|v: redis::Value| match v {
            redis::Value::Array(items) => match items.first() {
                Some(redis::Value::Int(n)) => *n as usize,
                _ => 0,
            },
            _ => 0,
        })
        .unwrap_or(0);
    assert_eq!(still_pending, 0, "stale entries were left pending forever");
}

#[tokio::test]
async fn a_republished_batch_is_not_broadcast_twice() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut rx = f.subscribe();

    let b = batch(1, vec![depth(1)]);
    publish(&mut r, &cfg, &b).await;
    drain(&mut f).await;
    assert_eq!(rx.try_recv().expect("the first copy").seq, 1);

    // Recovery: the engine replays its command log and publishes the same seq
    // again under a new stream id. A client that applied the delta once must
    // not be handed it a second time — its book would move twice on one trade.
    publish(&mut r, &cfg, &b).await;
    assert_eq!(
        drain(&mut f).await,
        1,
        "the entry was read and acknowledged"
    );
    assert!(
        rx.try_recv().is_err(),
        "a replayed batch was broadcast to live subscribers"
    );
}

#[tokio::test]
async fn a_batch_older_than_one_already_seen_is_not_broadcast() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut rx = f.subscribe();

    publish(&mut r, &cfg, &batch(9, vec![depth(9)])).await;
    drain(&mut f).await;
    assert_eq!(rx.try_recv().expect("seq 9").seq, 9);

    // A replay restarts from the snapshot, so the seqs that follow are ones
    // already broadcast. Going backwards is the signal to drop, not to deliver.
    publish(&mut r, &cfg, &batch(5, vec![depth(5)])).await;
    drain(&mut f).await;
    assert!(rx.try_recv().is_err());

    // And the feed picks straight back up once the replay passes the mark.
    publish(&mut r, &cfg, &batch(10, vec![depth(10)])).await;
    drain(&mut f).await;
    assert_eq!(rx.try_recv().expect("seq 10").seq, 10);
}

// ───────────────────────── bad input ─────────────────────────

#[tokio::test]
async fn an_undecodable_entry_is_acknowledged_and_does_not_wedge_the_stream() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut rx = f.subscribe();

    let _: String = r
        .xadd(
            &cfg.events_stream,
            "*",
            &[(FIELD_PAYLOAD, "{not json at all")],
        )
        .await
        .unwrap();
    publish(&mut r, &cfg, &batch(1, vec![depth(4)])).await;

    drain(&mut f).await;
    assert_eq!(
        rx.try_recv()
            .expect("the good batch behind the bad entry")
            .seq,
        1
    );
}

#[tokio::test]
async fn publishing_with_nobody_connected_is_not_an_error() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");

    // No subscribers at all. A market data feed at 4am is in this state, and it
    // must not be treated as a failure.
    publish(&mut r, &cfg, &batch(1, vec![depth(4)])).await;
    assert_eq!(drain(&mut f).await, 1);
}

#[tokio::test]
async fn a_batch_with_nothing_to_broadcast_is_still_consumed() {
    let cfg = test_config(&Uuid::new_v4().simple().to_string());
    let mut r = conn(&cfg).await;
    let mut f = Feed::boot(cfg.clone()).await.expect("feed boot");
    let mut rx = f.subscribe();

    publish(
        &mut r,
        &cfg,
        &batch(
            1,
            vec![Event::Deposited {
                user_id: Uuid::new_v4(),
                asset: "USDT".into(),
                amount: 1,
                available: 1,
            }],
        ),
    )
    .await;

    assert_eq!(
        drain(&mut f).await,
        1,
        "the entry was read and acknowledged"
    );
    assert!(rx.try_recv().is_err(), "but nothing was broadcast");
}
