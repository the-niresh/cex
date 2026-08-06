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
//!
//! ## Commands and queries run concurrently
//!
//! `state` is `Arc<Mutex<State>>`, shared with the query task `run()` spawns
//! (see `query_loop.rs`). Every command's `apply()` and `check_invariants()`
//! happen inside one lock section with no `.await` held across it, and so
//! does every query's `state.query()`. That is what makes it safe for a query
//! to be answered on a completely independent Redis connection and task,
//! running while the command loop sits in `XREAD BLOCK`, without ever
//! observing a half-applied command.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cex_core::state::{Snapshot, State};
use cex_core::MarketRegistry;
use cex_proto::{Command, EventBatch, Response, FIELD_PAYLOAD};
use redis::aio::MultiplexedConnection;
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::lock::EngineLock;
use crate::query_loop::{self, QueryLoopConfig, SharedState};
use crate::snapshot_store::SnapshotStore;
use crate::stream_id::StreamId;

pub struct Runner {
    cfg: Config,
    conn: MultiplexedConnection,
    /// Kept only to open a fresh connection for the query task in `run()` —
    /// deliberately not a clone of `conn` or the lock's connection. See the
    /// comment on `lock_conn` in `boot` for why sharing a multiplexed
    /// connection with a blocking command is the wrong move.
    client: redis::Client,
    store: SnapshotStore,
    state: SharedState,
    /// Last command id applied. Replay resumes from here.
    position: StreamId,
    applied_since_snapshot: usize,
    /// Proof that this process is the only engine on this command stream.
    /// Held for as long as the engine runs; losing it stops the engine.
    lock: EngineLock,
    /// The concurrent query task, once `run()` has started it. Aborted
    /// explicitly when the command loop stops, and again by `Drop` as a
    /// safety net if `run()`'s own future is cancelled from outside (e.g.
    /// `main.rs`'s `tokio::select!` against a stop signal) before it gets the
    /// chance to abort it itself.
    query_task: Option<JoinHandle<()>>,
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

        // The lease is renewed once a third of it has gone, and the loop can
        // sit in `XREAD` for `block_ms` between renewals. If the read can
        // outlast the renewal interval, a perfectly healthy engine loses its
        // own lock. That is knowable here, so it is refused here rather than
        // discovered at three in the morning.
        let refresh_interval = cfg.lock_ttl_ms / 3;
        if cfg.block_ms as u64 >= refresh_interval {
            anyhow::bail!(
                "block_ms ({}) must be below lock_ttl_ms / 3 ({}), or the engine \
                 will let its own lock lapse while waiting for commands",
                cfg.block_ms,
                refresh_interval
            );
        }

        // Before anything else. Rule 6 — exactly one engine per command stream —
        // is the one invariant whose violation corrupts state rather than
        // degrading service, because a second engine applies every command a
        // second time.
        // A connection of its own, deliberately not a clone of the one the read
        // loop uses. `MultiplexedConnection` pipelines over a single socket, so
        // a blocking `XREAD` sitting on it delays everything queued behind it —
        // which would mean releasing the lock on shutdown waits out `block_ms`,
        // and the replacement engine waits with it.
        let lock_conn = client
            .get_multiplexed_async_connection()
            .await
            .context("connecting to redis for the stream lock")?;

        let lock = EngineLock::acquire(
            lock_conn,
            &cfg.commands_stream,
            std::time::Duration::from_millis(cfg.lock_ttl_ms),
        )
        .await?;
        info!(
            stream = %cfg.commands_stream,
            engine = %lock.id(),
            "took the command stream lock"
        );

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
            client,
            store,
            state: Arc::new(Mutex::new(state)),
            position,
            applied_since_snapshot: 0,
            lock,
            query_task: None,
        })
    }

    /// This engine's hold on its command stream.
    pub fn lock(&mut self) -> &mut EngineLock {
        &mut self.lock
    }

    /// The current state, locked. Every existing call site (`.seq()`,
    /// `.check_invariants()`, `.query(..)`, `.open_order_ids()`, ...) keeps
    /// working unchanged: `MutexGuard<State>` derefs to `&State`.
    pub fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("state mutex poisoned")
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
            .xread_options(
                &[&self.cfg.commands_stream],
                &[&self.position.to_string()],
                &opts,
            )
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

    /// Answer every query currently waiting, and return how many were answered.
    ///
    /// A non-blocking `RPOP` drain against the same shared state the concurrent
    /// query task (`query_loop::run`) uses. `Runner::run` does not call this —
    /// production query serving is the concurrent task — but it stays as the
    /// deterministic, single-step way tests exercise query answering without
    /// spinning up a background task.
    pub async fn poll_queries(&mut self) -> Result<usize> {
        let mut answered = 0usize;
        loop {
            let payload: Option<String> = self
                .conn
                .rpop(&self.cfg.queries_queue, None)
                .await
                .context("popping the query queue")?;
            let Some(payload) = payload else { break };

            let Some(response) = query_loop::answer(&self.state, &payload) else {
                continue;
            };
            self.reply(response).await;
            answered += 1;
        }
        Ok(answered)
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

        // The whole apply-and-check happens inside one lock section, with no
        // `.await` in it. That is what makes it atomic with respect to the
        // query task: a query taking the lock either sees every effect of this
        // command or none of them, never a partial one.
        let outcome = {
            let mut state = self.state.lock().expect("state mutex poisoned");
            let outcome = state.apply(cmd);
            if outcome.is_ok() {
                debug_assert!(
                    state.check_invariants().is_ok(),
                    "invariants broken: {:?}",
                    state.check_invariants()
                );
            }
            outcome
        };

        match outcome {
            Ok(applied) => {
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
            .xadd(
                &self.cfg.events_stream,
                "*",
                &[(FIELD_PAYLOAD, json.as_str())],
            )
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
        // Locked just long enough to build the snapshot — `Snapshot::of` clones
        // what it needs. Encoding and the filesystem write happen after the
        // lock is released.
        let snap = {
            let state = self.state.lock().expect("state mutex poisoned");
            Snapshot::of(&state, self.position.to_string())
        };
        let seq = snap.state.seq();
        let path = self.store.save(&snap)?;
        self.applied_since_snapshot = 0;
        let pruned = self.store.prune()?;
        info!(
            path = %path.display(),
            position = %self.position,
            seq,
            pruned,
            "snapshot written"
        );
        Ok(())
    }

    /// Give up the command stream so a replacement can take it immediately.
    ///
    /// Without this every restart — including a planned deploy — would wait out
    /// the whole lease before the new engine could boot, because nothing else
    /// clears the key. A `kill -9` still falls back to lease expiry; this is
    /// what makes the *graceful* path cost nothing.
    ///
    /// Returns whether we were still the holder. `false` is not a failure: it
    /// means the lease had already lapsed and another engine legitimately owns
    /// the stream, in which case releasing deliberately does nothing.
    ///
    /// Deliberately does not snapshot. Callers that want a fast restart should
    /// call [`Runner::snapshot`] first, so the order is visible where it
    /// matters rather than hidden in here.
    pub async fn shutdown(&mut self) -> Result<bool> {
        let held = self.lock.release().await?;
        if held {
            info!(stream = %self.cfg.commands_stream, "released the command stream lock");
        } else {
            warn!(
                stream = %self.cfg.commands_stream,
                "lease had already lapsed; another engine may own this stream"
            );
        }
        Ok(held)
    }

    /// Run until the process is killed.
    ///
    /// Spawns the concurrent query task, then runs the command loop until it
    /// stops (only ever by losing the stream lock — `step()` errors are
    /// retried in place). The query task is aborted the moment the command
    /// loop stops: an engine that is no longer applying commands must not go
    /// on answering reads as though it still were.
    pub async fn run(&mut self) -> Result<()> {
        info!(
            stream = %self.cfg.commands_stream,
            from = %self.position,
            "engine running"
        );

        let query_conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .context("connecting to redis for the query loop")?;
        let query_cfg = QueryLoopConfig {
            queries_queue: self.cfg.queries_queue.clone(),
            responses_channel: self.cfg.responses_channel.clone(),
        };
        self.query_task = Some(tokio::spawn(query_loop::run(
            query_conn,
            query_cfg,
            Arc::clone(&self.state),
        )));

        let outcome = self.command_loop().await;

        if let Some(handle) = self.query_task.take() {
            handle.abort();
        }

        outcome
    }

    async fn command_loop(&mut self) -> Result<()> {
        loop {
            // Before touching anything, confirm we are still the engine. If the
            // lease has been taken, another process is applying these very
            // commands and the only safe thing left to do is stop: two engines
            // on one stream double-apply everything they see.
            self.lock.refresh_if_due().await?;

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

impl Drop for Runner {
    fn drop(&mut self) {
        if let Some(handle) = self.query_task.take() {
            handle.abort();
        }
    }
}
