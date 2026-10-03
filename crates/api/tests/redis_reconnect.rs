//! Redis outage and recovery without restarting the API.
//!
//! Needs the throwaway Redis from the CEX-8 test run:
//! `docker run -d --rm --name cex-test-redis -p 6391:6379 redis:7-alpine`

use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cex_api::routes::{build_router, AppState};
use cex_api::{Loopback, LoopbackConfig, Tokens, UserStore};
use cex_engine::config::Config as EngineConfig;
use cex_engine::runner::Runner;
use cex_persist::TestResources;
use http_body_util::BodyExt;
use tower::ServiceExt;
const REDIS_URL: &str = "redis://127.0.0.1:6391";
const DATABASE_URL: &str = "postgres://cex:cex@127.0.0.1:5442/cex";

fn docker_redis_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ensure_throwaway_redis() {
    let running = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", "cex-test-redis"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false);
    if running {
        return;
    }
    let exists = Command::new("docker")
        .args(["inspect", "cex-test-redis"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let status = if exists {
        Command::new("docker")
            .args(["start", "cex-test-redis"])
            .status()
            .expect("docker start")
    } else {
        let _ = Command::new("docker")
            .args(["rm", "-f", "cex-test-redis"])
            .status();
        Command::new("docker")
            .args([
                "run",
                "-d",
                "--name",
                "cex-test-redis",
                "-p",
                "6391:6379",
                "redis:7-alpine",
            ])
            .status()
            .expect("docker run")
    };
    assert!(status.success(), "could not start cex-test-redis");
}

async fn wait_for_redis() {
    for _ in 0..30 {
        if redis::Client::open(REDIS_URL)
            .and_then(|c| c.get_connection())
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("throwaway redis did not come back");
}

struct Harness {
    router: axum::Router,
    _engine: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
    _resources: TestResources,
}

impl Harness {
    async fn start() -> Self {
        ensure_throwaway_redis();
        cex_persist::require_safe_database_url();

        let resources = TestResources::new();
        let tag = resources.tag.clone();
        let dir = tempfile::tempdir().unwrap();

        let engine_cfg = EngineConfig {
            redis_url: REDIS_URL.into(),
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
            redis_url: REDIS_URL.into(),
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

        let users = UserStore::connect_to_schema(DATABASE_URL, &resources.schema)
            .await
            .expect("postgres");
        let loopback = Loopback::connect(loopback_cfg).await.expect("loopback");
        let tokens = Tokens::new(
            b"test secret for redis reconnect",
            Duration::from_secs(3600),
        );
        let history = cex_persist::HistoryStore::connect_to_schema(DATABASE_URL, &resources.schema)
            .await
            .expect("history");

        Harness {
            router: build_router(AppState::new(loopback, users, tokens, history)),
            _engine: engine,
            _dir: dir,
            _resources: resources,
        }
    }

    async fn get(&self, path: &str) -> (StatusCode, String) {
        let response = self
            .router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }
}

async fn wait_for_markets(h: &Harness) {
    for _ in 0..50 {
        if h.get("/markets").await.0 == StatusCode::OK {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("markets never became ready");
}

#[tokio::test]
async fn markets_survive_a_redis_restart_without_restarting_the_api() {
    let _lock = docker_redis_lock();
    let h = Harness::start().await;
    wait_for_markets(&h).await;

    let status = Command::new("docker")
        .args(["restart", "cex-test-redis"])
        .status()
        .expect("docker restart");
    assert!(status.success(), "docker restart cex-test-redis failed");

    wait_for_redis().await;

    let mut last = String::new();
    for _ in 0..50 {
        let (status, body) = h.get("/markets").await;
        if status == StatusCode::OK {
            return;
        }
        last = body;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("markets did not recover after redis restart, last body: {last}");
}

#[tokio::test]
async fn health_returns_503_while_redis_is_down_and_200_when_it_is_back() {
    let _lock = docker_redis_lock();
    let h = Harness::start().await;
    wait_for_markets(&h).await;

    let stop = Command::new("docker")
        .args(["stop", "cex-test-redis"])
        .status()
        .expect("docker stop");
    assert!(stop.success());

    tokio::time::sleep(Duration::from_millis(500)).await;
    let (status, body) = h.get("/health").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body: {body}");
    assert!(body.contains("redis unavailable"));

    let start = Command::new("docker")
        .args(["start", "cex-test-redis"])
        .status()
        .expect("docker start");
    assert!(start.success());

    wait_for_redis().await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let (status, body) = h.get("/health").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
}
