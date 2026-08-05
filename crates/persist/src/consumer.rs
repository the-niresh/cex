//! The persister loop.
//!
//! Read a batch of events, write them all in one transaction, acknowledge them.
//! That is the entire process.
//!
//! ## The ordering, and why it is that way round
//!
//! Write first, acknowledge second. The gap between them is a crash window, and
//! it is the safe way round: a crash there means Redis redelivers a batch that
//! is already in Postgres, and the `seq` guard turns the second write into a
//! no-op. The other order would lose history outright — acknowledge, crash,
//! and the batch is gone from the group with nothing on disk to show for it.
//!
//! ## Draining our own backlog first
//!
//! On boot the consumer reads its pending list (`XREADGROUP ... 0`) before it
//! asks for anything new (`>`). Those are entries Redis handed to this consumer
//! name and never saw acknowledged — the exact backlog a killed process leaves
//! behind. Going straight to `>` would step over all of them.

use anyhow::{Context, Result};
use cex_proto::{EventBatch, FIELD_PAYLOAD};
use redis::aio::MultiplexedConnection;
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use tracing::{debug, error, info};

use crate::config::Config;
use crate::store::HistoryStore;

pub struct Consumer {
    cfg: Config,
    conn: MultiplexedConnection,
    store: HistoryStore,
    /// While true, reads target this consumer's own unacknowledged backlog
    /// rather than new entries. Flips once that backlog comes back empty.
    draining_backlog: bool,
}

impl Consumer {
    /// Connect, make sure the consumer group exists, and prepare to resume.
    pub async fn boot(cfg: Config, store: HistoryStore) -> Result<Self> {
        let client = redis::Client::open(cfg.redis_url.as_str())
            .with_context(|| format!("opening redis at {}", cfg.redis_url))?;
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .context("connecting to redis")?;

        // `0` so a persister deployed after the exchange started still writes
        // the history that is already on the stream, rather than silently
        // beginning partway through. `MKSTREAM` so it can start before the
        // engine has published anything.
        let created: redis::RedisResult<String> = conn
            .xgroup_create_mkstream(&cfg.events_stream, &cfg.group, "0")
            .await;
        match created {
            Ok(_) => info!(group = %cfg.group, stream = %cfg.events_stream, "consumer group created"),
            Err(e) if is_busy_group(&e) => {
                debug!(group = %cfg.group, "consumer group already exists")
            }
            Err(e) => return Err(e).context("creating the consumer group"),
        }

        Ok(Consumer {
            cfg,
            conn,
            store,
            draining_backlog: true,
        })
    }

    /// Read one batch of entries, write them, acknowledge them.
    ///
    /// Returns how many stream entries were handled. Zero means there is
    /// nothing pending and the stream went quiet for `block_ms` — and *only*
    /// that. Running out of backlog is an internal mode change, so it falls
    /// through to new entries within the same call rather than surfacing as a
    /// zero a caller would read as "drained".
    pub async fn step(&mut self) -> Result<usize> {
        loop {
            let (ids, batches) = self.read().await?;

            if ids.is_empty() {
                if self.draining_backlog {
                    // Our own unacknowledged entries are all handled.
                    // Everything from here is new.
                    debug!("backlog clear, following the live stream");
                    self.draining_backlog = false;
                    continue;
                }
                return Ok(0);
            }

            // Write, then acknowledge. An error here leaves everything
            // unacknowledged, so Redis hands it all back and the retry starts
            // from a clean slate rather than a partial one.
            let written = self
                .store
                .write_batches(&batches)
                .await
                .context("writing history")?;

            let acked: i64 = self
                .conn
                .xack(&self.cfg.events_stream, &self.cfg.group, &ids)
                .await
                .context("acknowledging entries")?;

            debug!(entries = ids.len(), written, acked, "history written");
            return Ok(ids.len());
        }
    }

    /// One `XREADGROUP`, decoded. Returns the entry ids read and the batches
    /// that could be decoded from them.
    ///
    /// Every entry read is returned in `ids` whether it decoded or not: an
    /// entry nothing can ever be written for must still be acknowledged, or it
    /// blocks the group forever and stops all history behind it.
    async fn read(&mut self) -> Result<(Vec<String>, Vec<EventBatch>)> {
        let mut opts = StreamReadOptions::default()
            .group(&self.cfg.group, &self.cfg.consumer)
            .count(self.cfg.count);
        // Only block on new entries. The backlog is already there or it is not.
        if !self.draining_backlog {
            opts = opts.block(self.cfg.block_ms);
        }
        let cursor = if self.draining_backlog { "0" } else { ">" };

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
            "persister running"
        );
        loop {
            match self.step().await {
                Ok(0) => {}
                Ok(n) => debug!(entries = n, "batch persisted"),
                Err(e) => {
                    // Nothing was acknowledged, so nothing is lost — Redis will
                    // hand the same entries back. A batch that keeps failing
                    // stalls history loudly, which is the right failure: better
                    // a persister that pages you than one that quietly drops
                    // trades it could not write.
                    error!(error = %e, "write failed, retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        }
    }

    pub fn store(&self) -> &HistoryStore {
        &self.store
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }
}

/// Redis answers `XGROUP CREATE` on an existing group with a `BUSYGROUP` error.
/// That is the normal case on every boot after the first, not a failure.
fn is_busy_group(e: &redis::RedisError) -> bool {
    e.code() == Some("BUSYGROUP")
}
