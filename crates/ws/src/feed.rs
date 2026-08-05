//! The event stream, read once and copied to everyone.
//!
//! ## Why this group starts at the tail, and the persister's starts at the head
//!
//! History and live data want opposite things. `persist` creates its group at
//! `0` because a batch it never wrote is a hole in the record forever. This
//! process creates its at `$`, because market data has a shelf life measured in
//! milliseconds: replaying yesterday's depth deltas into a fresh connection
//! would not be catching up, it would be lying about the state of the book.
//!
//! For the same reason, entries left pending by a previous instance are cleared
//! without being broadcast. They are stale by definition — but leaving them in
//! the pending list forever would grow it without bound.

use anyhow::{Context, Result};
use cex_proto::{EventBatch, Seq, FIELD_PAYLOAD};
use redis::aio::MultiplexedConnection;
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::route::{route, Update};

pub struct Feed {
    cfg: Config,
    conn: MultiplexedConnection,
    tx: broadcast::Sender<Arc<Update>>,
    /// While true, reads target this consumer's stale pending list, which is
    /// acknowledged and discarded rather than broadcast.
    clearing_backlog: bool,
    /// Highest batch seq broadcast so far. The engine's counter is gap-free and
    /// monotonic, so anything at or below this has been seen already.
    ///
    /// In memory only, and deliberately: a restart starts at the tail of the
    /// stream anyway, so there is no earlier position for a durable mark to
    /// protect. What it does cover is the case that actually happens — the
    /// engine restarting and replaying underneath a feed that stayed up.
    highest_seq: Seq,
}

impl Feed {
    pub async fn boot(cfg: Config) -> Result<Self> {
        let client = redis::Client::open(cfg.redis_url.as_str())
            .with_context(|| format!("opening redis at {}", cfg.redis_url))?;
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .context("connecting to redis")?;

        // `$`: only entries published from now on. `MKSTREAM` so this can start
        // before the engine has published anything.
        let created: redis::RedisResult<String> = conn
            .xgroup_create_mkstream(&cfg.events_stream, &cfg.group, "$")
            .await;
        match created {
            Ok(_) => info!(group = %cfg.group, stream = %cfg.events_stream, "consumer group created"),
            Err(e) if is_busy_group(&e) => {
                debug!(group = %cfg.group, "consumer group already exists")
            }
            Err(e) => return Err(e).context("creating the consumer group"),
        }

        let (tx, _) = broadcast::channel(cfg.broadcast_capacity);

        Ok(Feed {
            cfg,
            conn,
            tx,
            clearing_backlog: true,
            highest_seq: 0,
        })
    }

    /// A new receiver, positioned at the current head. A subscriber never
    /// receives updates that predate its connection.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Update>> {
        self.tx.subscribe()
    }

    pub fn sender(&self) -> broadcast::Sender<Arc<Update>> {
        self.tx.clone()
    }

    /// Read one batch of entries and publish what they produce.
    ///
    /// Returns how many stream entries were handled. Zero means the stream went
    /// quiet for `block_ms` — and only that. Finishing the stale backlog is an
    /// internal mode change and falls through within the same call.
    pub async fn step(&mut self) -> Result<usize> {
        loop {
            let (ids, batches) = self.read().await?;

            if ids.is_empty() {
                if self.clearing_backlog {
                    debug!("stale backlog cleared, following the live stream");
                    self.clearing_backlog = false;
                    continue;
                }
                return Ok(0);
            }

            if self.clearing_backlog {
                // Stale by definition. Acknowledged so the pending list does not
                // grow without bound, and dropped rather than broadcast.
                warn!(
                    entries = ids.len(),
                    "discarding stale entries left pending by a previous instance"
                );
            } else {
                for batch in &batches {
                    // Recovery re-applies the command log, so the engine
                    // republishes batches it has already published — same seq,
                    // new stream id. `persist` deduplicates against a table;
                    // here the high-water mark is enough, and it has to be
                    // done: a depth delta applied twice moves a client's book
                    // twice on a single trade.
                    if batch.seq <= self.highest_seq {
                        debug!(seq = batch.seq, "replayed batch, not rebroadcasting");
                        continue;
                    }
                    self.highest_seq = batch.seq;

                    for update in route(batch) {
                        // An error means nobody is connected. That is the normal
                        // state of a market data feed at 4am, not a failure.
                        let _ = self.tx.send(Arc::new(update));
                    }
                }
            }

            // Acknowledged straight away: this process holds nothing durable,
            // so there is nothing a redelivery could usefully repair.
            let _: i64 = self
                .conn
                .xack(&self.cfg.events_stream, &self.cfg.group, &ids)
                .await
                .context("acknowledging entries")?;

            return Ok(ids.len());
        }
    }

    async fn read(&mut self) -> Result<(Vec<String>, Vec<EventBatch>)> {
        let mut opts = StreamReadOptions::default()
            .group(&self.cfg.group, &self.cfg.consumer)
            .count(self.cfg.count);
        if !self.clearing_backlog {
            opts = opts.block(self.cfg.block_ms);
        }
        let cursor = if self.clearing_backlog { "0" } else { ">" };

        let reply: Option<StreamReadReply> = self
            .conn
            .xread_options(&[&self.cfg.events_stream], &[cursor], &opts)
            .await
            .context("reading the event stream")?;

        let mut ids: Vec<String> = Vec::new();
        let mut batches: Vec<EventBatch> = Vec::new();

        let Some(reply) = reply else {
            return Ok((ids, batches));
        };

        for key in reply.keys {
            for entry in key.ids {
                // Acknowledged whether it decoded or not: an entry nothing can
                // be made of would otherwise sit in the pending list forever.
                ids.push(entry.id.clone());

                let payload: Option<String> = entry.get(FIELD_PAYLOAD);
                let Some(payload) = payload else {
                    error!(id = %entry.id, "event entry has no payload field, dropping");
                    continue;
                };
                match serde_json::from_str::<EventBatch>(&payload) {
                    Ok(batch) => batches.push(batch),
                    Err(e) => {
                        error!(id = %entry.id, error = %e, "undecodable event batch, dropping")
                    }
                }
            }
        }
        Ok((ids, batches))
    }

    /// Run until the process is killed.
    pub async fn run(&mut self) -> Result<()> {
        info!(
            stream = %self.cfg.events_stream,
            group = %self.cfg.group,
            consumer = %self.cfg.consumer,
            "feed running"
        );
        loop {
            match self.step().await {
                Ok(0) => {}
                Ok(n) => debug!(entries = n, "batch fanned out"),
                Err(e) => {
                    // Losing the read loop costs live data, not correctness —
                    // the durable record is `persist`'s job, not this one's.
                    error!(error = %e, "read failed, retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        }
    }
}

/// Redis answers `XGROUP CREATE` on an existing group with a `BUSYGROUP` error.
/// That is the normal case on every boot after the first, not a failure.
fn is_busy_group(e: &redis::RedisError) -> bool {
    e.code() == Some("BUSYGROUP")
}
