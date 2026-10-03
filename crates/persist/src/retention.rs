//! Delete history rows older than a configured window, in small batches.
//!
//! Off when `CEX_HISTORY_RETENTION_DAYS` is unset or zero. Turning it on is a
//! separate deployment decision because the first run deletes everything older
//! than the window.

use std::time::Duration;

use anyhow::Result;
use tracing::debug;

use crate::store::HistoryStore;

const BATCH: i64 = 5_000;
const PAUSE: Duration = Duration::from_millis(100);

/// Delete rows older than `retention_days`. Returns how many rows were removed.
pub async fn prune_old_history(store: &HistoryStore, retention_days: u32) -> Result<u64> {
    let mut total = 0u64;

    loop {
        let n = store
            .delete_history_older_than(retention_days, BATCH)
            .await?;
        if n == 0 {
            break;
        }
        total += n;
        debug!(deleted = n, total, "history retention batch");
        tokio::time::sleep(PAUSE).await;
    }

    Ok(total)
}

/// Read retention days from the environment. Zero means off.
pub fn retention_days_from_env() -> u32 {
    std::env::var("CEX_HISTORY_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
