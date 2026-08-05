//! The HTTP surface, end to end.
//!
//! These drive the real router against a real engine and a real Postgres — no
//! mocks. The router is called directly rather than over a socket, which keeps
//! them fast without weakening what they prove.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cex_api::routes::{build_router, AppState};
use cex_api::{Loopback, LoopbackConfig, Tokens, UserStore};
use cex_engine::config::Config as EngineConfig;
use cex_engine::runner::Runner;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::time::Duration;
use tower::ServiceExt;
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

/// A whole exchange: engine running in the background, API wired to it, and a
/// private Postgres schema so tests do not collide.
struct Harness {
    router: axum::Router,
    history: cex_persist::HistoryStore,
    _engine: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Harness {
        let tag = Uuid::new_v4().simple().to_string();
        let dir = tempfile::tempdir().unwrap();

        let engine_cfg = EngineConfig {
            redis_url: redis_url(),
            commands_stream: format!("test:{tag}:commands"),
            events_stream: format!("test:{tag}:events"),
            responses_channel: format!("test:{tag}:responses"),
            queries_queue: format!("test:{tag}:queries"),
            snapshot_dir: dir.path().to_path_buf(),
            snapshot_every: 1_000_000,
            snapshot_keep: 3,
            block_ms: 50,
            lock_ttl_ms: 30_000,
        };

        let loopback_cfg = LoopbackConfig {
            redis_url: redis_url(),
            commands_stream: engine_cfg.commands_stream.clone(),
            queries_queue: engine_cfg.queries_queue.clone(),
            responses_channel: engine_cfg.responses_channel.clone(),
            timeout: Duration::from_secs(10),
        };

        let mut runner = Runner::boot(engine_cfg).await.expect("engine boot");
        let engine = tokio::spawn(async move {
            loop {
                let _ = runner.poll_queries().await;
                let _ = runner.step().await;
            }
        });

        let users = UserStore::connect_to_schema(&database_url(), &format!("t{tag}"))
            .await
            .expect("postgres — is `docker compose up -d` running?");
        let loopback = Loopback::connect(loopback_cfg).await.expect("loopback");
        let tokens = Tokens::new(b"test secret for the route tests", Duration::from_secs(3600));

        let history = cex_persist::HistoryStore::connect_to_schema(
            &database_url(),
            &format!("t{tag}"),
        )
        .await
        .expect("postgres history schema");

        Harness {
            router: build_router(AppState::new(loopback, users, tokens, history.clone())),
            history,
            _engine: engine,
            _dir: dir,
        }
    }

    /// Write a trade print exactly as `persist` would, so the endpoint is read
    /// against real rows rather than a fixture shaped to suit it.
    async fn record_trade(&self, seq: u64, symbol: &str, price: i64, qty: i64) -> (Uuid, Uuid) {
        let maker = Uuid::new_v4();
        let taker = Uuid::new_v4();
        self.history
            .write_batches(&[cex_proto::EventBatch {
                seq,
                request_id: Uuid::new_v4(),
                events: vec![cex_proto::Event::Trades {
                    symbol: symbol.into(),
                    fills: vec![cex_proto::Fill {
                        symbol: symbol.into(),
                        price,
                        qty,
                        maker_order_id: 1,
                        taker_order_id: 2,
                        maker_user_id: maker,
                        taker_user_id: taker,
                        taker_side: cex_proto::Side::Buy,
                        notional: 50_000_000,
                        maker_fee: 10_000,
                        taker_fee: 50,
                    }],
                }],
            }])
            .await
            .expect("recording a trade");
        (maker, taker)
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let body: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, body)
    }

    async fn get(&self, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut b = Request::builder().method("GET").uri(path);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        self.call(b.body(Body::empty()).unwrap()).await
    }

    async fn send_with_key(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Value,
        key: &str,
    ) -> (StatusCode, Value) {
        let mut b = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("idempotency-key", key);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        self.call(b.body(Body::from(body.to_string())).unwrap()).await
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut b = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        self.call(b.body(Body::from(body.to_string())).unwrap()).await
    }

    /// Register a funded user and return their token.
    async fn user(&self, asset: &str, amount: i64) -> String {
        let username = format!("u{}", Uuid::new_v4().simple());
        let (status, body) = self
            .send(
                "POST",
                "/register",
                None,
                json!({"username": username, "password": "a-good-password"}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register failed: {body}");
        let token = body["token"].as_str().unwrap().to_string();

        let (status, body) = self
            .send(
                "POST",
                "/deposit",
                Some(&token),
                json!({"asset": asset, "amount": amount}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "deposit failed: {body}");
        token
    }
}

fn order(side: &str, price: i64, qty: i64) -> Value {
    json!({
        "symbol": SYM,
        "side": side,
        "order_type": "LIMIT",
        "time_in_force": "GTC",
        "price": price,
        "qty": qty,
    })
}

// ───────────────────────── open endpoints ─────────────────────────

#[tokio::test]
async fn health_reports_ok() {
    let h = Harness::start().await;
    let (status, body) = h.get("/health", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn markets_are_public() {
    let h = Harness::start().await;
    let (status, body) = h.get("/markets", None).await;

    assert_eq!(status, StatusCode::OK);
    let symbols: Vec<&str> = body["markets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["symbol"].as_str().unwrap())
        .collect();
    assert!(symbols.contains(&SYM));
}

#[tokio::test]
async fn depth_is_public_and_starts_empty() {
    let h = Harness::start().await;
    let (status, body) = h.get(&format!("/depth/{SYM}"), None).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["bids"].as_array().unwrap().is_empty());
    assert!(body["asks"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn depth_for_an_unknown_market_is_a_client_error() {
    let h = Harness::start().await;
    let (status, _) = h.get("/depth/NOPE_USDT", None).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ───────────────────────── registration and login ─────────────────────────

#[tokio::test]
async fn registering_returns_a_usable_token() {
    let h = Harness::start().await;
    let username = format!("u{}", Uuid::new_v4().simple());

    let (status, body) = h
        .send(
            "POST",
            "/register",
            None,
            json!({"username": username, "password": "a-good-password"}),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED);
    let token = body["token"].as_str().expect("a token");
    assert!(body["user_id"].as_str().is_some());

    // The token works immediately.
    let (status, _) = h.get("/balances", Some(token)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_duplicate_username_is_a_conflict() {
    let h = Harness::start().await;
    let username = format!("u{}", Uuid::new_v4().simple());
    let payload = json!({"username": username, "password": "a-good-password"});

    let (first, _) = h.send("POST", "/register", None, payload.clone()).await;
    assert_eq!(first, StatusCode::CREATED);

    let (second, _) = h.send("POST", "/register", None, payload).await;
    assert_eq!(second, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_weak_password_is_rejected() {
    let h = Harness::start().await;
    let (status, body) = h
        .send(
            "POST",
            "/register",
            None,
            json!({"username": "someone", "password": "short"}),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("8 characters"));
}

#[tokio::test]
async fn logging_in_with_the_right_password_returns_a_token() {
    let h = Harness::start().await;
    let username = format!("u{}", Uuid::new_v4().simple());
    h.send(
        "POST",
        "/register",
        None,
        json!({"username": username, "password": "a-good-password"}),
    )
    .await;

    let (status, body) = h
        .send(
            "POST",
            "/login",
            None,
            json!({"username": username, "password": "a-good-password"}),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].as_str().is_some());
}

#[tokio::test]
async fn logging_in_with_the_wrong_password_is_unauthorised() {
    let h = Harness::start().await;
    let username = format!("u{}", Uuid::new_v4().simple());
    h.send(
        "POST",
        "/register",
        None,
        json!({"username": username, "password": "a-good-password"}),
    )
    .await;

    let (status, _) = h
        .send(
            "POST",
            "/login",
            None,
            json!({"username": username, "password": "the-wrong-one"}),
        )
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ───────────────────────── auth middleware ─────────────────────────

#[tokio::test]
async fn a_protected_route_without_a_token_is_unauthorised() {
    let h = Harness::start().await;
    for (method, path) in [
        ("GET", "/balances"),
        ("GET", "/orders/open"),
        ("POST", "/orders"),
        ("POST", "/deposit"),
    ] {
        let (status, _) = h.send(method, path, None, json!({})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}

#[tokio::test]
async fn a_rubbish_token_is_unauthorised() {
    let h = Harness::start().await;
    for token in ["", "rubbish", "a.b.c"] {
        let (status, _) = h.get("/balances", Some(token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "token {token:?}");
    }
}

#[tokio::test]
async fn a_token_signed_by_someone_else_is_unauthorised() {
    let h = Harness::start().await;
    let forger = Tokens::new(b"not the servers secret at all", Duration::from_secs(3600));
    let forged = forger.issue(Uuid::new_v4()).unwrap();

    let (status, _) = h.get("/balances", Some(&forged)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ───────────────────────── money ─────────────────────────

#[tokio::test]
async fn a_deposit_shows_up_in_balances() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;

    let (status, body) = h.get("/balances", Some(&token)).await;

    assert_eq!(status, StatusCode::OK);
    let usdt = body["balances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["asset"] == "USDT")
        .expect("a USDT balance");
    assert_eq!(usdt["available"], 1_000_000_000i64);
    assert_eq!(usdt["locked"], 0);
}

#[tokio::test]
async fn placing_a_resting_order_locks_the_funds() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;

    let (status, body) = h
        .send("POST", "/orders", Some(&token), order("BUY", P50K, Q1))
        .await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(body["order_id"].as_u64().unwrap() > 0);
    assert_eq!(body["status"], "OPEN");

    let (_, body) = h.get("/balances", Some(&token)).await;
    let usdt = body["balances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["asset"] == "USDT")
        .unwrap();
    assert_eq!(usdt["locked"], 50_000_000i64, "notional should be locked");
}

#[tokio::test]
async fn an_order_the_user_cannot_fund_is_rejected() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000).await; // nowhere near enough

    let (status, body) = h
        .send("POST", "/orders", Some(&token), order("BUY", P50K, Q1))
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("insufficient"));
}

#[tokio::test]
async fn a_misaligned_price_is_rejected() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;

    let (status, _) = h
        .send("POST", "/orders", Some(&token), order("BUY", P50K + 1, Q1))
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ───────────────────────── orders ─────────────────────────

#[tokio::test]
async fn open_orders_lists_only_your_own() {
    let h = Harness::start().await;
    let alice = h.user("USDT", 1_000_000_000).await;
    let bob = h.user("USDT", 1_000_000_000).await;

    h.send("POST", "/orders", Some(&alice), order("BUY", P50K, Q1))
        .await;

    let (status, body) = h.get("/orders/open", Some(&alice)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["orders"].as_array().unwrap().len(), 1);

    let (_, body) = h.get("/orders/open", Some(&bob)).await;
    assert!(body["orders"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn cancelling_an_order_releases_the_funds() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;

    let (_, body) = h
        .send("POST", "/orders", Some(&token), order("BUY", P50K, Q1))
        .await;
    let id = body["order_id"].as_u64().unwrap();

    let (status, _) = h
        .send("DELETE", &format!("/orders/{id}"), Some(&token), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = h.get("/balances", Some(&token)).await;
    let usdt = body["balances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["asset"] == "USDT")
        .unwrap();
    assert_eq!(usdt["locked"], 0);
    assert_eq!(usdt["available"], 1_000_000_000i64);
}

#[tokio::test]
async fn you_cannot_cancel_someone_elses_order() {
    let h = Harness::start().await;
    let alice = h.user("USDT", 1_000_000_000).await;
    let bob = h.user("USDT", 1_000_000_000).await;

    let (_, body) = h
        .send("POST", "/orders", Some(&alice), order("BUY", P50K, Q1))
        .await;
    let id = body["order_id"].as_u64().unwrap();

    let (status, _) = h
        .send("DELETE", &format!("/orders/{id}"), Some(&bob), json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Alice's order is untouched.
    let (_, body) = h.get("/orders/open", Some(&alice)).await;
    assert_eq!(body["orders"].as_array().unwrap().len(), 1);
}

// ───────────────────────── a whole trade ─────────────────────────

#[tokio::test]
async fn two_users_trade_and_both_balances_settle() {
    // The end-to-end proof: everything from HTTP to the order book and back.
    let h = Harness::start().await;
    let buyer = h.user("USDT", 1_000_000_000).await;
    let seller = h.user("BTC", 100_000_000).await;

    // Seller rests an ask at 49,000.
    let (status, _) = h
        .send(
            "POST",
            "/orders",
            Some(&seller),
            order("SELL", 49_000_000_000, Q1),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Buyer crosses it with a limit at 50,000.
    let (status, body) = h
        .send("POST", "/orders", Some(&buyer), order("BUY", P50K, Q1))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "FILLED");
    assert_eq!(
        body["avg_price"], 49_000_000_000i64,
        "must fill at the maker's price, not the taker's"
    );

    // Buyer: paid 49 USDT, holds BTC less the taker fee.
    let (_, body) = h.get("/balances", Some(&buyer)).await;
    let arr = body["balances"].as_array().unwrap();
    let usdt = arr.iter().find(|b| b["asset"] == "USDT").unwrap();
    let btc = arr.iter().find(|b| b["asset"] == "BTC").unwrap();
    assert_eq!(usdt["available"], 1_000_000_000i64 - 49_000_000);
    assert_eq!(usdt["locked"], 0, "price improvement was refunded");
    assert_eq!(btc["available"], Q1 - Q1 * 5 / 10_000);

    // Seller: delivered BTC, holds USDT less the maker fee.
    let (_, body) = h.get("/balances", Some(&seller)).await;
    let arr = body["balances"].as_array().unwrap();
    let usdt = arr.iter().find(|b| b["asset"] == "USDT").unwrap();
    assert_eq!(usdt["available"], 49_000_000 - 49_000_000 * 2 / 10_000);

    // The book is empty again.
    let (_, body) = h.get(&format!("/depth/{SYM}"), None).await;
    assert!(body["bids"].as_array().unwrap().is_empty());
    assert!(body["asks"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_book_shows_resting_orders_from_every_user() {
    let h = Harness::start().await;
    let alice = h.user("USDT", 10_000_000_000).await;
    let bob = h.user("BTC", 100_000_000).await;

    h.send("POST", "/orders", Some(&alice), order("BUY", 49_000_000_000, Q1))
        .await;
    h.send("POST", "/orders", Some(&bob), order("SELL", P50K, Q1))
        .await;

    let (_, body) = h.get(&format!("/depth/{SYM}"), None).await;
    assert_eq!(body["bids"][0][0], 49_000_000_000i64);
    assert_eq!(body["asks"][0][0], P50K);
}

// ───────────────────────── trade history ─────────────────────────

#[tokio::test]
async fn trades_are_public_and_newest_first() {
    let h = Harness::start().await;
    h.record_trade(1, SYM, P50K, Q1).await;
    h.record_trade(2, SYM, P50K + 1_000_000, Q1 * 2).await;

    let (status, body) = h.get(&format!("/trades/{SYM}"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let trades = body["trades"].as_array().expect("a trades array");
    assert_eq!(trades.len(), 2);
    assert_eq!(
        trades[0]["seq"], 2,
        "the most recent print must come first: {body}"
    );
    assert_eq!(trades[0]["price"], P50K + 1_000_000);
    assert_eq!(trades[0]["qty"], Q1 * 2);
    assert_eq!(trades[0]["taker_side"], "BUY");
    assert!(
        trades[0]["timestamp_ms"].as_i64().unwrap_or(0) > 0,
        "a trade feed without a time is nearly useless: {body}"
    );
}

/// The same rule the websocket feed keeps: a public endpoint says what traded,
/// never who.
#[tokio::test]
async fn the_trades_endpoint_never_says_who_traded() {
    let h = Harness::start().await;
    let (maker, taker) = h.record_trade(1, SYM, P50K, Q1).await;

    let (_, body) = h.get(&format!("/trades/{SYM}"), None).await;
    let raw = body.to_string();

    assert!(
        !raw.contains(&maker.to_string()),
        "the response named the maker: {raw}"
    );
    assert!(
        !raw.contains(&taker.to_string()),
        "the response named the taker: {raw}"
    );
    assert!(
        !raw.contains("maker_order_id") && !raw.contains("taker_order_id"),
        "order ids identify traders across requests: {raw}"
    );
}

#[tokio::test]
async fn trades_from_another_market_are_not_included() {
    let h = Harness::start().await;
    h.record_trade(1, SYM, P50K, Q1).await;
    h.record_trade(2, "ETH_USDT", 3_000_000_000, Q1).await;

    let (_, body) = h.get(&format!("/trades/{SYM}"), None).await;
    let trades = body["trades"].as_array().unwrap();
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0]["seq"], 1);
}

#[tokio::test]
async fn a_market_that_has_never_traded_returns_an_empty_list() {
    let h = Harness::start().await;
    let (status, body) = h.get(&format!("/trades/{SYM}"), None).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["trades"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn trades_for_an_unknown_market_is_a_client_error() {
    // Consistent with `/depth`. Answering a typo with an empty list looks
    // identical to a market that simply has not traded yet.
    let h = Harness::start().await;
    let (status, _) = h.get("/trades/NOT_A_MARKET", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn trades_honours_a_limit() {
    let h = Harness::start().await;
    for seq in 1..=5 {
        h.record_trade(seq, SYM, P50K + seq as i64, Q1).await;
    }

    let (_, body) = h.get(&format!("/trades/{SYM}?limit=2"), None).await;
    let trades = body["trades"].as_array().unwrap();
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0]["seq"], 5, "still newest first");
}

#[tokio::test]
async fn an_unbounded_limit_is_capped_rather_than_served() {
    let h = Harness::start().await;
    h.record_trade(1, SYM, P50K, Q1).await;

    // Left unchecked this is a way to ask one HTTP request to drag the entire
    // trade history of the exchange through Postgres.
    let (status, body) = h
        .get(&format!("/trades/{SYM}?limit=100000000"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["limit"], 500, "the cap should be reported back: {body}");
}

#[tokio::test]
async fn a_nonsense_limit_is_a_client_error() {
    let h = Harness::start().await;
    let (status, _) = h.get(&format!("/trades/{SYM}?limit=banana"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ───────────────────────── idempotency ─────────────────────────

/// A 504 is genuinely ambiguous: the command is already on the durable log, so
/// a timeout is not proof nothing happened. A caller that simply retries must
/// not be charged twice.

#[tokio::test]
async fn a_retried_order_with_the_same_key_places_one_order() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;

    let (s1, b1) = h
        .send_with_key("POST", "/orders", Some(&token), order("BUY", P50K, Q1), "abc-123")
        .await;
    let (s2, b2) = h
        .send_with_key("POST", "/orders", Some(&token), order("BUY", P50K, Q1), "abc-123")
        .await;

    assert_eq!(s1, StatusCode::CREATED, "{b1}");
    assert_eq!(s2, StatusCode::CREATED, "{b2}");
    assert_eq!(
        b1["order_id"], b2["order_id"],
        "the retry must name the order that already exists, not a new one"
    );

    let (_, open) = h.get("/orders/open", Some(&token)).await;
    assert_eq!(
        open["orders"].as_array().unwrap().len(),
        1,
        "the retry placed a second order: {open}"
    );
}

#[tokio::test]
async fn a_retried_deposit_with_the_same_key_credits_once() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000).await;

    let body = json!({"asset": "USDT", "amount": 500});
    h.send_with_key("POST", "/deposit", Some(&token), body.clone(), "dep-1")
        .await;
    h.send_with_key("POST", "/deposit", Some(&token), body, "dep-1")
        .await;

    let (_, balances) = h.get("/balances", Some(&token)).await;
    let usdt = balances["balances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["asset"] == "USDT")
        .cloned()
        .expect("a USDT balance");
    assert_eq!(
        usdt["available"], 1_500,
        "the retry credited twice: {balances}"
    );
}

#[tokio::test]
async fn without_a_key_two_identical_requests_are_two_orders() {
    // Idempotency is opt-in. Two deliberate identical orders must both stand.
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;

    h.send("POST", "/orders", Some(&token), order("BUY", P50K, Q1))
        .await;
    h.send("POST", "/orders", Some(&token), order("BUY", P50K, Q1))
        .await;

    let (_, open) = h.get("/orders/open", Some(&token)).await;
    assert_eq!(open["orders"].as_array().unwrap().len(), 2);
}

/// The key is the caller's, not the exchange's.
#[tokio::test]
async fn two_users_may_choose_the_same_idempotency_key() {
    let h = Harness::start().await;
    let alice = h.user("USDT", 1_000_000_000).await;
    let bob = h.user("USDT", 1_000_000_000).await;

    let (sa, ba) = h
        .send_with_key("POST", "/orders", Some(&alice), order("BUY", P50K, Q1), "same")
        .await;
    let (sb, bb) = h
        .send_with_key("POST", "/orders", Some(&bob), order("BUY", P50K, Q1), "same")
        .await;

    assert_eq!(sa, StatusCode::CREATED, "{ba}");
    assert_eq!(sb, StatusCode::CREATED, "{bb}");
    assert_ne!(
        ba["order_id"], bb["order_id"],
        "bob was handed alice's order because they picked the same key"
    );

    for token in [&alice, &bob] {
        let (_, open) = h.get("/orders/open", Some(token)).await;
        assert_eq!(open["orders"].as_array().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn a_different_key_is_a_different_order() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;

    h.send_with_key("POST", "/orders", Some(&token), order("BUY", P50K, Q1), "one")
        .await;
    h.send_with_key("POST", "/orders", Some(&token), order("BUY", P50K, Q1), "two")
        .await;

    let (_, open) = h.get("/orders/open", Some(&token)).await;
    assert_eq!(open["orders"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn an_empty_idempotency_key_is_rejected() {
    // Silently ignoring it would leave the caller believing they are protected.
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;

    let (status, _) = h
        .send_with_key("POST", "/orders", Some(&token), order("BUY", P50K, Q1), "   ")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_over_long_idempotency_key_is_rejected() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;
    let huge = "k".repeat(5_000);

    let (status, _) = h
        .send_with_key("POST", "/orders", Some(&token), order("BUY", P50K, Q1), &huge)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_retried_cancel_is_harmless() {
    let h = Harness::start().await;
    let token = h.user("USDT", 1_000_000_000).await;
    let (_, placed) = h
        .send("POST", "/orders", Some(&token), order("BUY", P50K, Q1))
        .await;
    let id = placed["order_id"].as_u64().unwrap();

    let (s1, _) = h
        .send_with_key("DELETE", &format!("/orders/{id}"), Some(&token), json!({}), "c-1")
        .await;
    let (s2, b2) = h
        .send_with_key("DELETE", &format!("/orders/{id}"), Some(&token), json!({}), "c-1")
        .await;

    assert_eq!(s1, StatusCode::OK);
    assert_eq!(
        s2,
        StatusCode::OK,
        "the retry of a cancel should report the original outcome, not a fresh \
         error about an order that is no longer open: {b2}"
    );
}
