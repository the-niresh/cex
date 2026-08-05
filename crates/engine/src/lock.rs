//! The engine boot lock.
//!
//! Design rule 6 — exactly one engine per command stream — was, until now, only
//! a sentence in the README. The engine reads commands with plain `XREAD`,
//! which is not a consumer group, so a second instance does not split the work
//! with the first: it reads *every* command and applies all of them again. Two
//! engines on one stream means every deposit credited twice and every order
//! matched twice. It is the one failure left in this system that corrupts state
//! rather than degrading service.
//!
//! ## A lease, not a guarantee
//!
//! This is a lease with a timeout, and it is worth being honest about what that
//! does and does not buy. It reliably stops the accidental second start — the
//! thing that actually happens, and did. It cannot make double-application
//! impossible: a process paused long enough for its lease to lapse may not
//! notice until it wakes, and in that window another engine can legitimately
//! hold the stream. Closing that gap completely needs a fencing token that
//! Redis itself checks on every write, which is a much larger change to the
//! command log. What is here narrows the window to one lease and fails loudly
//! when it is hit, rather than corrupting state in silence.
//!
//! ## Why the key is derived from the stream name
//!
//! The invariant is per command stream, not per exchange. Two engines running
//! against two different streams is a legitimate deployment; two against one is
//! the bug. Deriving `cex:commands:lock` from `cex:commands` makes the lock and
//! the thing it protects impossible to configure apart.

use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use std::time::{Duration, Instant};
use tracing::debug;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("the command stream {stream} is already owned by engine {holder}")]
    Held { stream: String, holder: String },
    #[error("lost the lock on {0}: another engine owns this command stream now")]
    Lost(String),
    #[error("redis: {0}")]
    Redis(String),
}

fn redis_err(e: redis::RedisError) -> LockError {
    LockError::Redis(e.to_string())
}

/// The lock guarding one command stream.
pub fn lock_key(commands_stream: &str) -> String {
    format!("{commands_stream}:lock")
}

/// Extend the lease, but only while we are still the holder.
///
/// The check and the write have to be one atomic step. A `GET` followed by a
/// separate `PEXPIRE` can interleave with another engine taking the lock, and
/// the loser would then cheerfully extend the winner's lease.
const REFRESH: &str = r#"
if redis.call("GET", KEYS[1]) == ARGV[1] then
    return redis.call("PEXPIRE", KEYS[1], ARGV[2])
else
    return 0
end
"#;

/// Release, but only while we are still the holder.
///
/// Same reasoning, and the consequence of getting it wrong is worse: a stale
/// engine deleting the key would unlock the stream out from under the engine
/// that legitimately owns it, and let a third one in.
const RELEASE: &str = r#"
if redis.call("GET", KEYS[1]) == ARGV[1] then
    return redis.call("DEL", KEYS[1])
else
    return 0
end
"#;

pub struct EngineLock {
    conn: MultiplexedConnection,
    key: String,
    stream: String,
    /// This engine's identity. Unique per process, so no engine can release or
    /// extend another's lock even by mistake.
    id: String,
    ttl: Duration,
    last_refresh: Instant,
}

impl EngineLock {
    /// Take the lock for `commands_stream`, or fail if another engine holds it.
    ///
    /// There is deliberately no waiting and no retry. A second engine starting
    /// is an operator error, and the useful response is to stop immediately
    /// with a message naming who has the stream — not to sit in a loop until
    /// the incumbent dies and then quietly double up.
    pub async fn acquire(
        mut conn: MultiplexedConnection,
        commands_stream: &str,
        ttl: Duration,
    ) -> Result<Self, LockError> {
        let key = lock_key(commands_stream);
        let id = format!("engine-{}", Uuid::new_v4());

        let taken: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(&id)
            .arg("NX")
            .arg("PX")
            .arg(ttl.as_millis() as u64)
            .query_async(&mut conn)
            .await
            .map_err(redis_err)?;

        if taken.is_none() {
            // Whoever holds it may have let go between the SET and this read,
            // in which case there is nothing useful to name.
            let holder: Option<String> = conn.get(&key).await.map_err(redis_err)?;
            return Err(LockError::Held {
                stream: commands_stream.to_string(),
                holder: holder.unwrap_or_else(|| "an engine that has since exited".into()),
            });
        }

        Ok(EngineLock {
            conn,
            key,
            stream: commands_stream.to_string(),
            id,
            ttl,
            last_refresh: Instant::now(),
        })
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Extend the lease.
    ///
    /// Failing here is not a transient hiccup to be retried — it means another
    /// engine owns this stream and is applying the same commands. The caller
    /// must stop.
    pub async fn refresh(&mut self) -> Result<(), LockError> {
        let extended: i64 = redis::Script::new(REFRESH)
            .key(&self.key)
            .arg(&self.id)
            .arg(self.ttl.as_millis() as u64)
            .invoke_async(&mut self.conn)
            .await
            .map_err(redis_err)?;

        if extended == 0 {
            return Err(LockError::Lost(self.stream.clone()));
        }
        self.last_refresh = Instant::now();
        debug!(stream = %self.stream, "lease extended");
        Ok(())
    }

    /// Extend the lease if a third of it has gone. Returns whether it did.
    ///
    /// A third leaves room for two renewals to fail or be delayed before the
    /// lease actually lapses, which is the difference between surviving a
    /// momentary stall and stopping the exchange over one slow round trip.
    pub async fn refresh_if_due(&mut self) -> Result<bool, LockError> {
        if self.last_refresh.elapsed() < self.ttl / 3 {
            return Ok(false);
        }
        self.refresh().await?;
        Ok(true)
    }

    /// Give up the lock. Returns whether we were still the holder.
    ///
    /// `false` is not an error — it means the lease had already lapsed and
    /// someone else has the stream. The important part is that this never
    /// deletes a lock it does not own.
    pub async fn release(&mut self) -> Result<bool, LockError> {
        let deleted: i64 = redis::Script::new(RELEASE)
            .key(&self.key)
            .arg(&self.id)
            .invoke_async(&mut self.conn)
            .await
            .map_err(redis_err)?;
        Ok(deleted == 1)
    }
}

impl std::fmt::Debug for EngineLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineLock")
            .field("key", &self.key)
            .field("id", &self.id)
            .field("ttl", &self.ttl)
            .finish()
    }
}
