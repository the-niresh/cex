//! Read requests.
//!
//! Queries never touch the command log — logging a read would bloat the stream
//! and slow every future replay for no benefit. They arrive on their own list
//! and are answered on the same response channel commands use, keyed by
//! `request_id`.
//!
//! They are answered from the same state the command loop owns, so a read is
//! always consistent with the last applied command.

use cex_engine::config::Config;
use cex_engine::runner::Runner;
use cex_proto::{
    Command, OrderType, Query, Response, ResponseResult, Side, TimeInForce, FIELD_PAYLOAD,
};
use redis::AsyncCommands;
use uuid::Uuid;

const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn test_config(dir: &std::path::Path) -> Config {
    let tag = Uuid::new_v4().simple().to_string();
    Config {
        redis_url: std::env::var("CEX_REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6390".into()),
        commands_stream: format!("test:{tag}:commands"),
        events_stream: format!("test:{tag}:events"),
        responses_channel: format!("test:{tag}:responses"),
        queries_queue: format!("test:{tag}:queries"),
        snapshot_dir: dir.to_path_buf(),
        snapshot_every: 1_000_000,
        snapshot_keep: 3,
        block_ms: 150,
    }
}

async fn conn(cfg: &Config) -> redis::aio::MultiplexedConnection {
    redis::Client::open(cfg.redis_url.as_str())
        .expect("redis client")
        .get_multiplexed_async_connection()
        .await
        .expect("redis — is `docker compose up -d` running?")
}

async fn send_command(c: &mut redis::aio::MultiplexedConnection, cfg: &Config, cmd: &Command) {
    let json = serde_json::to_string(cmd).unwrap();
    let _: String = c
        .xadd(&cfg.commands_stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
        .await
        .unwrap();
}

async fn send_query(c: &mut redis::aio::MultiplexedConnection, cfg: &Config, q: &Query) {
    let json = serde_json::to_string(q).unwrap();
    let _: i64 = c.lpush(&cfg.queries_queue, json).await.unwrap();
}

async fn drain_commands(runner: &mut Runner) {
    while runner.step().await.unwrap() > 0 {}
}

/// Answer every queued query and collect the replies for `wanted`.
///
/// Command replies share this channel, so the caller is identified by
/// `request_id` and anything else is skipped — which is precisely what the API
/// will do with its own pending-request map.
async fn answer_queries(
    runner: &mut Runner,
    sub: &mut redis::aio::PubSub,
    wanted: &[Uuid],
) -> Vec<Response> {
    let handled = runner.poll_queries().await.unwrap();
    assert_eq!(
        handled,
        wanted.len(),
        "engine answered the wrong number of queries"
    );

    let mut out = Vec::new();
    let deadline = std::time::Duration::from_secs(5);
    let mut stream = sub.on_message();
    while out.len() < wanted.len() {
        let msg = tokio::time::timeout(deadline, stream.next())
            .await
            .expect("timed out waiting for a response")
            .expect("subscription closed");
        let payload: String = msg.get_payload().unwrap();
        let r: Response = serde_json::from_str(&payload).unwrap();
        if wanted.contains(&r.request_id) {
            out.push(r);
        }
    }
    out
}

use futures_util::StreamExt;

async fn subscriber(cfg: &Config) -> redis::aio::PubSub {
    let client = redis::Client::open(cfg.redis_url.as_str()).unwrap();
    let mut sub = client.get_async_pubsub().await.unwrap();
    sub.subscribe(&cfg.responses_channel).await.unwrap();
    sub
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

fn body(r: &Response) -> &cex_proto::ResponseBody {
    match &r.result {
        ResponseResult::Ok { data } => data,
        ResponseResult::Err { error } => panic!("expected success, got error: {error}"),
    }
}

// ───────────────────────── answering ─────────────────────────

#[tokio::test]
async fn a_balance_query_is_answered_on_the_response_channel() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let alice = Uuid::new_v4();

    send_command(&mut c, &cfg, &deposit(alice, "USDT", 1_234)).await;
    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain_commands(&mut runner).await;

    let rid = Uuid::new_v4();
    send_query(
        &mut c,
        &cfg,
        &Query::Balances {
            request_id: rid,
            user_id: alice,
        },
    )
    .await;

    let responses = answer_queries(&mut runner, &mut sub, &[rid]).await;
    assert_eq!(responses[0].request_id, rid, "reply must echo the request id");

    match body(&responses[0]) {
        cex_proto::ResponseBody::Balances(v) => {
            assert_eq!(v.len(), 1);
            assert_eq!(v[0].asset, "USDT");
            assert_eq!(v[0].available, 1_234);
        }
        other => panic!("expected Balances, got {other:?}"),
    }
}

#[tokio::test]
async fn a_depth_query_reflects_commands_already_applied() {
    // The consistency guarantee: a read sees everything applied before it.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let alice = Uuid::new_v4();

    send_command(&mut c, &cfg, &deposit(alice, "USDT", 1_000_000_000)).await;
    send_command(&mut c, &cfg, &limit(alice, Side::Buy, P50K, Q1)).await;
    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain_commands(&mut runner).await;

    let rid = Uuid::new_v4();
    send_query(
        &mut c,
        &cfg,
        &Query::Depth {
            request_id: rid,
            symbol: SYM.into(),
            limit: None,
        },
    )
    .await;

    let responses = answer_queries(&mut runner, &mut sub, &[rid]).await;
    match body(&responses[0]) {
        cex_proto::ResponseBody::Depth(d) => {
            assert_eq!(d.bids, vec![[P50K, Q1]]);
            assert!(d.asks.is_empty());
        }
        other => panic!("expected Depth, got {other:?}"),
    }
}

#[tokio::test]
async fn a_markets_query_lists_the_tradable_pairs() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let mut runner = Runner::boot(cfg.clone()).await.unwrap();

    let rid = Uuid::new_v4();
    send_query(&mut c, &cfg, &Query::Markets { request_id: rid }).await;

    let responses = answer_queries(&mut runner, &mut sub, &[rid]).await;
    match body(&responses[0]) {
        cex_proto::ResponseBody::Markets(m) => {
            let symbols: Vec<&str> = m.iter().map(|x| x.symbol.as_str()).collect();
            assert!(symbols.contains(&"BTC_USDT"));
            assert!(m.iter().all(|x| x.tick_size > 0 && x.lot_size > 0));
        }
        other => panic!("expected Markets, got {other:?}"),
    }
}

#[tokio::test]
async fn several_queued_queries_are_all_answered() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let alice = Uuid::new_v4();

    send_command(&mut c, &cfg, &deposit(alice, "USDT", 10)).await;
    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain_commands(&mut runner).await;

    let mut ids = Vec::new();
    for _ in 0..3 {
        let rid = Uuid::new_v4();
        ids.push(rid);
        send_query(
            &mut c,
            &cfg,
            &Query::Balances {
                request_id: rid,
                user_id: alice,
            },
        )
        .await;
    }

    let responses = answer_queries(&mut runner, &mut sub, &ids).await;
    assert_eq!(responses.len(), 3);
}

// ───────────────────────── failure handling ─────────────────────────

#[tokio::test]
async fn a_query_for_an_unknown_market_is_answered_with_an_error() {
    // The caller must get a reply either way, or it sits until it times out.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let mut runner = Runner::boot(cfg.clone()).await.unwrap();

    let rid = Uuid::new_v4();
    send_query(
        &mut c,
        &cfg,
        &Query::Depth {
            request_id: rid,
            symbol: "NOPE_USDT".into(),
            limit: None,
        },
    )
    .await;

    let responses = answer_queries(&mut runner, &mut sub, &[rid]).await;
    assert_eq!(responses[0].request_id, rid);
    match &responses[0].result {
        ResponseResult::Err { error } => assert!(error.contains("NOPE_USDT")),
        ResponseResult::Ok { .. } => panic!("expected an error for an unknown market"),
    }
}

#[tokio::test]
async fn a_malformed_query_is_dropped_without_stopping_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let alice = Uuid::new_v4();

    let _: i64 = c.lpush(&cfg.queries_queue, "{ not json").await.unwrap();
    send_query(
        &mut c,
        &cfg,
        &Query::Balances {
            request_id: Uuid::new_v4(),
            user_id: alice,
        },
    )
    .await;

    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    // Two items consumed; only the valid one produces a reply.
    let handled = runner.poll_queries().await.unwrap();
    assert_eq!(handled, 1, "the good query must still be answered");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), sub.on_message().next())
        .await
        .expect("timed out")
        .unwrap();
    let payload: String = msg.get_payload().unwrap();
    let _: Response = serde_json::from_str(&payload).unwrap();
}

#[tokio::test]
async fn an_empty_query_queue_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut runner = Runner::boot(cfg).await.unwrap();

    assert_eq!(runner.poll_queries().await.unwrap(), 0);
}

// ───────────────────────── the log stays clean ─────────────────────────

#[tokio::test]
async fn answering_queries_does_not_append_to_the_command_log() {
    // The design rule this whole split exists for.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let alice = Uuid::new_v4();

    send_command(&mut c, &cfg, &deposit(alice, "USDT", 10)).await;
    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain_commands(&mut runner).await;

    let before: usize = c.xlen(&cfg.commands_stream).await.unwrap();

    let mut ids = Vec::new();
    for _ in 0..5 {
        let rid = Uuid::new_v4();
        ids.push(rid);
        send_query(
            &mut c,
            &cfg,
            &Query::Balances {
                request_id: rid,
                user_id: alice,
            },
        )
        .await;
    }
    answer_queries(&mut runner, &mut sub, &ids).await;

    let after: usize = c.xlen(&cfg.commands_stream).await.unwrap();
    assert_eq!(after, before, "queries leaked into the command log");
}

#[tokio::test]
async fn answering_queries_does_not_advance_the_engine_sequence() {
    // A read is not a state transition.
    let dir = tempfile::tempdir().unwrap();
    let cfg = test_config(dir.path());
    let mut c = conn(&cfg).await;
    let mut sub = subscriber(&cfg).await;
    let alice = Uuid::new_v4();

    send_command(&mut c, &cfg, &deposit(alice, "USDT", 10)).await;
    let mut runner = Runner::boot(cfg.clone()).await.unwrap();
    drain_commands(&mut runner).await;
    let seq_before = runner.state().seq();

    let rid = Uuid::new_v4();
    send_query(
        &mut c,
        &cfg,
        &Query::Balances {
            request_id: rid,
            user_id: alice,
        },
    )
    .await;
    answer_queries(&mut runner, &mut sub, &[rid]).await;

    assert_eq!(runner.state().seq(), seq_before);
}
