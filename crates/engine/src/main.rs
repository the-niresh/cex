//! The engine process.

use anyhow::Result;
use cex_engine::{Config, Runner};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = Config::from_env();
    let mut runner = Runner::boot(cfg).await?;
    runner.run().await
}
