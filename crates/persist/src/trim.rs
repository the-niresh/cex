//! Trim the events stream once every consumer group has confirmed past a point.
//!
//! An entry is safe to drop only when both `cex:persist` and `cex:ws` have
//! acknowledged everything up to and including it. Pending entries in either
//! group block the trim point at the oldest unacknowledged id.

use anyhow::{Context, Result};
use redis::aio::ConnectionManager;
use tracing::debug;

/// Groups that must both have confirmed an entry before it can be trimmed.
pub const EVENT_GROUPS: [&str; 2] = ["cex:persist", "cex:ws"];

/// Trim `cex:events` entries that every group in `groups` has confirmed.
///
/// Returns how many entries were removed, or zero when there is nothing safe to
/// trim yet.
pub async fn trim_confirmed_events(
    conn: &mut ConnectionManager,
    stream: &str,
    groups: &[&str],
) -> Result<u64> {
    let Some(trim_before) = confirmed_trim_point(conn, stream, groups).await? else {
        return Ok(0);
    };

    let removed: i64 = redis::cmd("XTRIM")
        .arg(stream)
        .arg("MINID")
        .arg("~")
        .arg(&trim_before)
        .query_async(conn)
        .await
        .context("trimming the events stream")?;

    if removed > 0 {
        debug!(stream, trim_before, removed, "trimmed confirmed events");
    }
    Ok(removed as u64)
}

/// The smallest stream id that may still be needed by any of the groups.
async fn confirmed_trim_point(
    conn: &mut ConnectionManager,
    stream: &str,
    groups: &[&str],
) -> Result<Option<String>> {
    let mut limits: Vec<String> = Vec::new();

    for group in groups {
        let info: Vec<redis::Value> = redis::cmd("XINFO")
            .arg("GROUPS")
            .arg(stream)
            .query_async(conn)
            .await
            .with_context(|| format!("reading group info for {group}"))?;

        let mut found = false;
        for entry in &info {
            let map = value_map(entry)?;
            let name = map.get("name").and_then(value_str);
            if name.as_deref() != Some(*group) {
                continue;
            }
            found = true;

            let pending: i64 = map.get("pending").and_then(value_i64).unwrap_or(0);

            if pending > 0 {
                let oldest: Vec<redis::Value> = redis::cmd("XPENDING")
                    .arg(stream)
                    .arg(group)
                    .arg("-")
                    .arg("+")
                    .arg(1)
                    .query_async(conn)
                    .await
                    .with_context(|| format!("reading pending for {group}"))?;
                if let Some(id) = oldest
                    .first()
                    .and_then(value_array)
                    .and_then(|a| a.first())
                    .and_then(value_str)
                {
                    limits.push(id.clone());
                }
            } else {
                let last = map
                    .get("last-delivered-id")
                    .and_then(value_str)
                    .filter(|id| *id != "0-0");
                if let Some(id) = last {
                    limits.push(next_stream_id(&id));
                }
            }
            break;
        }

        if !found {
            // Group does not exist yet; nothing is confirmed.
            return Ok(None);
        }
    }

    if limits.is_empty() {
        return Ok(None);
    }

    limits.sort_by(|a, b| compare_stream_ids(a, b));
    Ok(Some(limits[0].clone()))
}

fn next_stream_id(id: &str) -> String {
    let (ms, seq) = parse_id(id);
    if seq < u64::MAX {
        format!("{}-{}", ms, seq + 1)
    } else {
        format!("{}-0", ms + 1)
    }
}

fn compare_stream_ids(a: &str, b: &str) -> std::cmp::Ordering {
    let (ams, aseq) = parse_id(a);
    let (bms, bseq) = parse_id(b);
    ams.cmp(&bms).then(aseq.cmp(&bseq))
}

fn parse_id(s: &str) -> (u64, u64) {
    let (ms, seq) = s.split_once('-').unwrap_or((s, "0"));
    (ms.parse().unwrap_or(0), seq.parse().unwrap_or(0))
}

fn value_map(v: &redis::Value) -> Result<std::collections::HashMap<String, redis::Value>> {
    let items = value_array(v).context("expected array")?;
    let mut map = std::collections::HashMap::new();
    let mut iter = items.iter();
    while let Some(key) = iter.next() {
        if let Some(val) = iter.next() {
            if let Some(k) = value_str(key) {
                map.insert(k, val.clone());
            }
        }
    }
    Ok(map)
}

fn value_array(v: &redis::Value) -> Option<&[redis::Value]> {
    match v {
        redis::Value::Array(a) => Some(a),
        _ => None,
    }
}

fn value_str(v: &redis::Value) -> Option<String> {
    match v {
        redis::Value::BulkString(b) => Some(String::from_utf8_lossy(b).into_owned()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

fn value_i64(v: &redis::Value) -> Option<i64> {
    match v {
        redis::Value::Int(n) => Some(*n),
        _ => None,
    }
}
