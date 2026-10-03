//! Trim the command stream after a snapshot makes entries replay-safe to drop.

use anyhow::{Context, Result};
use redis::aio::ConnectionManager;
use tracing::debug;

use crate::stream_id::StreamId;

/// Trim command log entries older than `position`, which the newest snapshot
/// has already captured.
pub async fn trim_commands_before(
    conn: &mut ConnectionManager,
    stream: &str,
    position: StreamId,
) -> Result<u64> {
    let keep_from = trim_min_id(position);
    let removed: i64 = redis::cmd("XTRIM")
        .arg(stream)
        .arg("MINID")
        .arg("~")
        .arg(&keep_from)
        .query_async(conn)
        .await
        .context("trimming the command stream")?;

    if removed > 0 {
        debug!(stream, keep_from, removed, "trimmed applied commands");
    }
    Ok(removed as u64)
}

fn trim_min_id(position: StreamId) -> String {
    if position.seq < u64::MAX {
        StreamId {
            ms: position.ms,
            seq: position.seq + 1,
        }
        .to_string()
    } else {
        StreamId {
            ms: position.ms + 1,
            seq: 0,
        }
        .to_string()
    }
}
