//! The API process.

use std::time::Duration;

use anyhow::{Context, Result};
use cex_api::{build_router, AppState, Loopback, LoopbackConfig, Tokens, UserStore};

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let bind = env_or("CEX_API_BIND", "0.0.0.0:8080");
    let database_url = env_or("CEX_DATABASE_URL", "postgres://cex:cex@127.0.0.1:5442/cex");

    // A signing secret that changed on restart would invalidate every issued
    // token, so refuse to start rather than quietly generate one.
    let secret = std::env::var("CEX_JWT_SECRET")
        .context("CEX_JWT_SECRET is not set. Generate one with: openssl rand -hex 32")?;
    if secret.len() < 32 {
        anyhow::bail!("CEX_JWT_SECRET must be at least 32 characters");
    }

    let loopback = Loopback::connect(LoopbackConfig {
        redis_url: env_or("CEX_REDIS_URL", "redis://127.0.0.1:6390"),
        commands_stream: env_or("CEX_COMMANDS_STREAM", cex_proto::STREAM_COMMANDS),
        queries_queue: env_or("CEX_QUERIES_QUEUE", cex_proto::QUEUE_QUERIES),
        responses_channel: env_or("CEX_RESPONSES_CHANNEL", cex_proto::CHANNEL_RESPONSES),
        timeout: Duration::from_secs(5),
    })
    .await
    .map_err(|e| anyhow::anyhow!("connecting to redis: {e}"))?;

    let users = UserStore::connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("connecting to postgres: {e}"))?;

    let tokens = Tokens::new(secret.as_bytes(), Duration::from_secs(24 * 3600));
    let app = build_router(AppState::new(loopback, users, tokens));

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "api listening");

    axum::serve(listener, app).await.context("serving")
}
