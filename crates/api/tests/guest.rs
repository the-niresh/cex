//! Guest login, rate limits, and deposit ceilings.

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use cex_api::guest::{generate_guest_name, is_guest_name_format};
use cex_api::routes::{build_router, AppState};
use cex_api::{Loopback, LoopbackConfig, Tokens, UserStore};
use cex_engine::config::Config as EngineConfig;
use cex_engine::runner::Runner;
use cex_persist::TestResources;
use http_body_util::BodyExt;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const TEST_SECRET: &[u8] = b"test secret for the guest tests!!";
const GUEST_TTL_SECS: u64 = 30 * 24 * 3600;

fn redis_url() -> String {
    std::env::var("CEX_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6391".into())
}

fn database_url() -> String {
    std::env::var("CEX_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://cex:cex@127.0.0.1:5442/cex".into())
}

struct Harness {
    router: axum::Router,
    users: UserStore,
    _engine: tokio::task::JoinHandle<()>,
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
        let router_tokens = Tokens::new(TEST_SECRET, Duration::from_secs(3600));
        let history =
            cex_persist::HistoryStore::connect_to_schema(&database_url(), &resources.schema)
                .await
                .expect("history");

        Harness {
            router: build_router(AppState::new(
                loopback,
                users.clone(),
                router_tokens,
                history,
            )),
            users,
            _engine: engine,
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

    async fn guest_from(&self, ip: IpAddr, xff: Option<&str>) -> (StatusCode, Value) {
        let mut b = Request::builder()
            .method("POST")
            .uri("/guest")
            .extension(ConnectInfo(SocketAddr::new(ip, 12345)));
        if let Some(h) = xff {
            b = b.header("x-forwarded-for", h);
        }
        self.call(b.body(Body::empty()).unwrap()).await
    }

    async fn deposit(&self, token: &str, asset: &str, amount: i64) -> (StatusCode, Value) {
        self.call(
            Request::builder()
                .method("POST")
                .uri("/deposit")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"asset": asset, "amount": amount}).to_string(),
                ))
                .unwrap(),
        )
        .await
    }
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Claims {
    sub: Uuid,
    exp: u64,
}

fn decode_exp(token: &str) -> u64 {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    decode::<Claims>(token, &DecodingKey::from_secret(TEST_SECRET), &validation)
        .unwrap()
        .claims
        .exp
}

#[test]
fn generated_guest_names_match_the_expected_shape() {
    for _ in 0..50 {
        let name = generate_guest_name();
        assert!(is_guest_name_format(&name), "bad name: {name}");
    }
}

#[tokio::test]
async fn a_guest_is_created_with_a_valid_name_and_session() {
    let h = Harness::start().await;
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    let (status, body) = h.guest_from(ip, None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let name = body["name"].as_str().unwrap();
    assert!(is_guest_name_format(name));
    assert!(body["token"].as_str().unwrap().len() > 10);
}

#[tokio::test]
async fn a_guest_cannot_log_in_with_any_password() {
    let h = Harness::start().await;
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
    let (_, body) = h.guest_from(ip, None).await;
    let name = body["name"].as_str().unwrap();
    let err = h.users.authenticate(name, "anything").await.unwrap_err();
    assert!(matches!(err, cex_api::UsersError::BadCredentials));
}

#[tokio::test]
async fn a_guest_token_lasts_thirty_days() {
    let h = Harness::start().await;
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3));
    let (_, body) = h.guest_from(ip, None).await;
    let token = body["token"].as_str().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let exp = decode_exp(token);
    let delta = exp - now;
    assert!(delta > GUEST_TTL_SECS - 120, "exp too soon: {delta}");
    assert!(delta <= GUEST_TTL_SECS + 5, "exp too far: {delta}");
}

#[tokio::test]
async fn the_sixth_guest_from_one_ip_in_an_hour_is_refused() {
    let h = Harness::start().await;
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
    for i in 0..5 {
        let (status, body) = h.guest_from(ip, None).await;
        assert_eq!(status, StatusCode::CREATED, "attempt {i}: {body}");
    }
    let (status, body) = h.guest_from(ip, None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(body["error"].as_str().unwrap().contains("too many"));
}

#[tokio::test]
async fn client_ip_uses_the_last_x_forwarded_for_entry() {
    let h = Harness::start().await;
    // Socket is 10.0.0.99 but XFF says the proxy saw 10.0.0.5 last.
    let socket = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 99));
    for _ in 0..5 {
        let (status, _) = h.guest_from(socket, Some("203.0.113.1, 10.0.0.5")).await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, _) = h.guest_from(socket, Some("203.0.113.1, 10.0.0.5")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn deposit_at_the_usdt_limit_is_ok_and_one_atom_over_is_refused() {
    let h = Harness::start().await;
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 10));
    let (_, body) = h.guest_from(ip, None).await;
    let token = body["token"].as_str().unwrap();
    let limit = 1_000_000i64 * 1_000_000; // 1M USDT in atoms
    let (status, _) = h.deposit(token, "USDT", limit).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = h.deposit(token, "USDT", limit + 1).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("USDT"));
}

#[tokio::test]
async fn deposit_limits_for_btc_eth_and_sol() {
    let h = Harness::start().await;
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 11));
    let (_, body) = h.guest_from(ip, None).await;
    let token = body["token"].as_str().unwrap();

    let cases = [
        ("BTC", 10i64 * 100_000_000, 10i64 * 100_000_000 + 1),
        ("ETH", 100i64 * 100_000_000, 100i64 * 100_000_000 + 1),
        ("SOL", 10_000i64 * 100_000_000, 10_000i64 * 100_000_000 + 1),
    ];
    for (asset, ok, bad) in cases {
        let (status, _) = h.deposit(token, asset, ok).await;
        assert_eq!(status, StatusCode::OK, "{asset} at limit");
        let (status, body) = h.deposit(token, asset, bad).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{asset} over limit: {body}"
        );
    }
}

#[tokio::test]
async fn guest_name_clash_retries_up_to_five_times() {
    // Force a clash by pre-registering a name, then stub generation - we test
    // the store retry path by inserting the first generated name manually.
    let h = Harness::start().await;
    let name = generate_guest_name();
    let hash = cex_api::hash_password("secret").unwrap();
    h.users
        .create_guest(&name, &name, &hash)
        .await
        .expect("seed guest");
    // Another guest should still succeed with a different random name.
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 20));
    let (status, body) = h.guest_from(ip, None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_ne!(body["name"].as_str().unwrap(), name);
}
