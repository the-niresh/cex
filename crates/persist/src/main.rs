//! The persister process.

use anyhow::{Context, Result};
use cex_persist::{Config, Consumer, HistoryStore};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = Config::from_env();
    let store = HistoryStore::connect_to_schema(&cfg.database_url, &cfg.schema)
        .await
        .context("connecting to postgres")?;
    let mut consumer = Consumer::boot(cfg, store).await?;
    consumer.run().await
}
