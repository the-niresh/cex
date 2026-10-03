//! Funding splits and re-auth against a throwaway stack.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cex_api::routes::{build_router, AppState};
use cex_api::server::serve;
use cex_api::{Loopback, LoopbackConfig, Tokens, UserStore};
use cex_engine::config::Config as EngineConfig;
use cex_engine::runner::Runner;
use cex_loadgen::venue::{fund, place_limit, register, with_reauth, Backoff, Funding, StateLogger};
use cex_persist::TestResources;
use cex_proto::deposit::{deposit_chunks, deposit_limit_atoms};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const TEST_TIMEOUT: Duration = Duration::from_secs(30);
const SYM: &str = "BTC_USDT";
const P50K: i64 = 50_000_000_000;
const Q1: i64 = 100_000;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6391".into())
}

fn database_url() -> String {
    std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into())
}

struct Harness {
    host: String,
    router: axum::Router,
    _engine: tokio::task::JoinHandle<()>,
    _server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
    _resources: TestResources,
}

impl Harness {
    async fn start() -> Self {
        let resources = TestResources::new();
        let tag = resources.tag.clone();
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

        let users = UserStore::connect_to_schema(&database_url(), &resources.schema)
            .await
            .expect("postgres");
        let loopback = Loopback::connect(loopback_cfg).await.expect("loopback");
        let tokens = Tokens::new(
            b"loadgen venue test secret!!!!!!",
            Duration::from_secs(3600),
        );
        let history =
            cex_persist::HistoryStore::connect_to_schema(&database_url(), &resources.schema)
                .await
                .expect("history");

        let router = build_router(AppState::new(loopback, users, tokens, history));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server_router = router.clone();
        let server = tokio::spawn(async move {
            serve(listener, server_router).await.expect("serve");
        });

        Harness {
            host: format!("http://{addr}"),
            router,
            _engine: engine,
            _server: server,
            _dir: dir,
            _resources: resources,
        }
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: Value = if body.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&body)
                .unwrap_or(json!({ "raw": String::from_utf8_lossy(&body) }))
        };
        (status, json)
    }
}

async fn with_timeout<F, T>(label: &str, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(TEST_TIMEOUT, f)
        .await
        .unwrap_or_else(|_| panic!("{label} hung for {}s", TEST_TIMEOUT.as_secs()))
}

#[tokio::test]
async fn funding_splits_at_each_asset_limit() {
    let h = Harness::start().await;
    let http = reqwest::Client::new();

    let btc_limit = deposit_limit_atoms("BTC").unwrap();
    let target = btc_limit * 3 + 1;
    let chunks = deposit_chunks("BTC", target);
    assert_eq!(chunks, vec![btc_limit, btc_limit, btc_limit, 1]);

    let account = register(&http, &h.host, "fundtest")
        .await
        .expect("register");
    fund(&http, &h.host, &account, 0, target)
        .await
        .expect("fund in chunks");

    let (status, body) = h
        .call(
            Request::builder()
                .uri("/balances")
                .header("authorization", format!("Bearer {}", account.token()))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let btc = body["balances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["asset"].as_str() == Some("BTC"))
        .expect("BTC balance");
    assert_eq!(btc["available"].as_i64(), Some(target));
}

#[tokio::test]
async fn a_401_leads_to_login_and_the_next_order_succeeds() {
    let h = Harness::start().await;
    let http = reqwest::Client::new();

    let mut account = register(&http, &h.host, "reauth").await.expect("register");
    let funding = Funding {
        usdt: deposit_limit_atoms("USDT").unwrap(),
        btc: deposit_limit_atoms("BTC").unwrap(),
    };
    fund(&http, &h.host, &account, funding.usdt, funding.btc)
        .await
        .expect("fund");

    account.token = "not-a-real-token".to_string();

    let mut logger = StateLogger::new();
    let mut backoff = Backoff::new();
    let host = h.host.clone();
    let order_id = with_timeout("reauth order", async {
        with_reauth(
            &http,
            &host,
            &mut account,
            funding,
            &mut logger,
            &mut backoff,
            |who| {
                let http = &http;
                let host = host.clone();
                let who = who.clone();
                async move { place_limit(http, &host, &who, SYM, "BUY", P50K, Q1).await }
            },
        )
        .await
    })
    .await
    .expect("order after reauth");

    assert!(order_id > 0);

    let (status, body) = h
        .call(
            Request::builder()
                .uri("/orders/open")
                .header("authorization", format!("Bearer {}", account.token()))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let open = body["orders"]
        .as_array()
        .expect("open orders list")
        .iter()
        .any(|o| o["order_id"].as_u64() == Some(order_id));
    assert!(open, "order {order_id} should be open after reauth");
}
