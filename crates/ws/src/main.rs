//! The market data process.

use anyhow::{Context, Result};
use cex_auth::Tokens;
use cex_ws::{build_router, AppState, Config, Feed};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // The same secret the api signs with. Without it this process cannot verify
    // anyone, so refuse to start rather than serve a feed that silently has no
    // private channels.
    let secret = std::env::var("CEX_JWT_SECRET").context("CEX_JWT_SECRET must be set")?;
    let tokens = Tokens::new(secret.as_bytes(), Duration::from_secs(24 * 3600));

    let cfg = Config::from_env();
    let bind = cfg.bind.clone();
    let mut feed = Feed::boot(cfg).await?;
    let state = AppState::new(feed.sender(), tokens);

    tokio::spawn(async move {
        if let Err(e) = feed.run().await {
            tracing::error!(error = %e, "feed stopped");
        }
    });

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "market data listening");
    axum::serve(listener, build_router(state))
        .await
        .context("serving")
}
