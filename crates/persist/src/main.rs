//! The persister process.

use std::time::Duration;

use anyhow::{Context, Result};
use cex_persist::{retention, trim, Config, Consumer, HistoryStore};
use cex_proto::STREAM_EVENTS;

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
    let mut consumer = Consumer::boot(cfg.clone(), store).await?;

    let retention_days = retention::retention_days_from_env();
    if retention_days > 0 {
        let store = consumer.store().clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                if let Err(e) = retention::prune_old_history(&store, retention_days).await {
                    tracing::error!(error = format!("{:#}", e), "history retention failed");
                }
            }
        });
    }

    if cfg.events_stream == STREAM_EVENTS {
        let redis_url = cfg.redis_url.clone();
        let stream = cfg.events_stream.clone();
        tokio::spawn(async move {
            let client = redis::Client::open(redis_url.as_str()).expect("redis client");
            let mut conn = redis::aio::ConnectionManager::new(client)
                .await
                .expect("redis connection");
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                if let Err(e) =
                    trim::trim_confirmed_events(&mut conn, &stream, &trim::EVENT_GROUPS).await
                {
                    tracing::error!(error = format!("{:#}", e), "events trim failed");
                }
            }
        });
    }

    consumer.run().await
}
