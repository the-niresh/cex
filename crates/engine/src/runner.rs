//! The engine loop.
//!
//! Read a command, apply it, publish what happened, reply, occasionally
//! snapshot. That is the entire process.
//!
//! ## Why plain `XREAD` and not a consumer group
//!
//! A consumer group tracks its own cursor server-side, which would compete with
//! the snapshot for authority over "where are we". Here the snapshot *is* the
//! cursor: it records the last applied id, and on boot we resume from exactly
//! that point. One source of truth for position, and replay stays exact.
//!
//! ## Ordering
//!
//! The position advances only after a command has been fully applied and
//! published. A crash mid-command means that command is replayed on restart —
//! at-least-once delivery, which is why downstream consumers deduplicate on
//! `seq`.

use anyhow::{Context, Result};
use cex_core::state::{Snapshot, State};
use cex_core::MarketRegistry;
use cex_proto::{Command, EventBatch, Response, FIELD_PAYLOAD};
use redis::aio::MultiplexedConnection;
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::snapshot_store::SnapshotStore;
use crate::stream_id::StreamId;

pub struct Runner {
    cfg: Config,
    conn: MultiplexedConnection,
    store: SnapshotStore,
    state: State,
    /// Last command id applied. Replay resumes from here.
    position: StreamId,
    applied_since_snapshot: usize,
}

impl Runner {
    /// Connect, load the newest usable snapshot, and prepare to resume.
    pub async fn boot(cfg: Config) -> Result<Self> {
        let client = redis::Client::open(cfg.redis_url.as_str())
            .with_context(|| format!("opening redis at {}", cfg.redis_url))?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .context("connecting to redis")?;

        let store = SnapshotStore::new(cfg.snapshot_dir.clone(), cfg.snapshot_keep);

        let (state, position) = match store.load_latest()? {
            Some(snap) => {
                let pos = StreamId::parse(&snap.last_stream_id).unwrap_or(StreamId::ZERO);
                info!(resume_from = %pos, seq = snap.state.seq(), "restored from snapshot");
                (snap.state, pos)
            }
            None => {
                info!("no usable snapshot, replaying the log from the beginning");
                (State::new(MarketRegistry::with_defaults()), StreamId::ZERO)
            }
        };

        Ok(Runner {
            cfg,
            conn,
            store,
            state,
            position,
            applied_since_snapshot: 0,
        })
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn position(&self) -> StreamId {
        self.position
    }

    pub fn store(&self) -> &SnapshotStore {
        &self.store
    }

    /// Read and apply one batch. Returns how many commands were applied.
    ///
    /// Zero means the stream went quiet for `block_ms`, which is the normal idle
    /// case and also how tests know they have drained everything.
    pub async fn step(&mut self) -> Result<usize> {
        let opts = StreamReadOptions::default()
            .block(self.cfg.block_ms)
            .count(256);

        let reply: Option<StreamReadReply> = self
            .conn
            .xread_options(&[&self.cfg.commands_stream], &[&self.position.to_string()], &opts)
            .await
            .context("reading the command stream")?;

        let Some(reply) = reply else { return Ok(0) };

        let mut applied = 0usize;
        for key in reply.keys {
            for entry in key.ids {
                let Some(id) = StreamId::parse(&entry.id) else {
                    warn!(id = %entry.id, "unparseable stream id, skipping");
                    continue;
                };

                let payload: Option<String> = entry.get(FIELD_PAYLOAD);
                let Some(payload) = payload else {
                    warn!(id = %entry.id, "entry has no payload field, skipping");
                    self.position = id;
                    continue;
                };

                self.handle(&payload).await;

                // Advance only after the command is fully applied and published.
                self.position = id;
                applied += 1;
                self.applied_since_snapshot += 1;
            }
        }

        if self.applied_since_snapshot >= self.cfg.snapshot_every {
            self.snapshot().await?;
        }

        Ok(applied)
    }

    /// Apply one command and publish its outcome.
    ///
    /// A malformed or rejected command is answered with an error and does not
    /// stop the loop — one bad message must never wedge the exchange.
    async fn handle(&mut self, payload: &str) {
        let cmd: Command = match serde_json::from_str(payload) {
            Ok(c) => c,
            Err(e) => {
                error!(error = %e, "undecodable command, skipping");
                return;
            }
        };
        let request_id = cmd.request_id();

        match self.state.apply(cmd) {
            Ok(applied) => {
                debug_assert!(
                    self.state.check_invariants().is_ok(),
                    "invariants broken: {:?}",
                    self.state.check_invariants()
                );

                if !applied.events.is_empty() {
                    let batch = EventBatch {
                        seq: applied.seq,
                        request_id,
                        events: applied.events,
                    };
                    self.publish_events(&batch).await;
                }
                self.reply(Response::ok(request_id, applied.response)).await;
            }
            Err(e) => {
                debug!(error = %e, "command rejected");
                self.reply(Response::err(request_id, e.to_string())).await;
            }
        }
    }

    async fn publish_events(&mut self, batch: &EventBatch) {
        let Ok(json) = serde_json::to_string(batch) else {
            error!("could not encode event batch");
            return;
        };
        let res: redis::RedisResult<String> = self
            .conn
            .xadd(&self.cfg.events_stream, "*", &[(FIELD_PAYLOAD, json.as_str())])
            .await;
        if let Err(e) = res {
            // The command is already applied and is on the durable log, so the
            // events can be regenerated by replay. Losing this publish degrades
            // the live feed, it does not corrupt state.
            error!(error = %e, "failed to publish events");
        }
    }

    async fn reply(&mut self, response: Response) {
        let Ok(json) = serde_json::to_string(&response) else {
            error!("could not encode response");
            return;
        };
        let res: redis::RedisResult<i64> = self
            .conn
            .publish(&self.cfg.responses_channel, json.as_str())
            .await;
        if let Err(e) = res {
            // The caller will time out. Better than wedging the loop.
            warn!(error = %e, "failed to publish response");
        }
    }

    /// Write a snapshot at the current position and prune old ones.
    pub async fn snapshot(&mut self) -> Result<()> {
        let snap = Snapshot::of(&self.state, self.position.to_string());
        let path = self.store.save(&snap)?;
        self.applied_since_snapshot = 0;
        let pruned = self.store.prune()?;
        info!(
            path = %path.display(),
            position = %self.position,
            seq = self.state.seq(),
            pruned,
            "snapshot written"
        );
        Ok(())
    }

    /// Run until the process is killed.
    pub async fn run(&mut self) -> Result<()> {
        info!(
            stream = %self.cfg.commands_stream,
            from = %self.position,
            "engine running"
        );
        loop {
            match self.step().await {
                Ok(0) => {}
                Ok(n) => debug!(applied = n, position = %self.position, "batch applied"),
                Err(e) => {
                    error!(error = %e, "read failed, retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        }
    }
}
