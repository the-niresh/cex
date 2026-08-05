use cex_proto::STREAM_EVENTS;

/// Everything the persister needs from its environment.
///
/// Stream, group and schema names are all configurable so tests can run against
/// a real Redis and a real Postgres without colliding with each other or with a
/// running persister.
#[derive(Debug, Clone)]
pub struct Config {
    pub redis_url: String,
    pub database_url: String,
    /// Postgres schema the history tables live in.
    pub schema: String,
    pub events_stream: String,
    /// Consumer group name. Its cursor is Redis's to track, not ours — that is
    /// the whole reason this service reads with `XREADGROUP` and the engine
    /// does not.
    pub group: String,
    /// This instance's name within the group.
    ///
    /// **It must be stable across restarts.** Redis holds unacknowledged
    /// entries against the consumer name that received them, so a name that
    /// changed on every boot would orphan its own backlog: the entries would
    /// stay pending forever and their history would never be written. Give each
    /// deployed instance its own fixed name.
    pub consumer: String,
    /// Maximum entries to claim in one read.
    pub count: usize,
    /// How long `XREADGROUP` blocks waiting for new events, in milliseconds.
    pub block_ms: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            // Non-default ports: the compose file deliberately avoids clashing
            // with any other Redis or Postgres on the box.
            redis_url: "redis://127.0.0.1:6390".into(),
            database_url: "postgres://cex:cex@127.0.0.1:5442/cex".into(),
            schema: "public".into(),
            events_stream: STREAM_EVENTS.into(),
            group: "cex:persist".into(),
            consumer: "persist-1".into(),
            count: 256,
            block_ms: 5_000,
        }
    }
}

impl Config {
    /// Read overrides from the environment, falling back to the defaults above.
    pub fn from_env() -> Self {
        let d = Config::default();
        Config {
            redis_url: env_or("CEX_REDIS_URL", d.redis_url),
            database_url: env_or("CEX_DATABASE_URL", d.database_url),
            schema: env_or("CEX_PERSIST_SCHEMA", d.schema),
            events_stream: env_or("CEX_EVENTS_STREAM", d.events_stream),
            group: env_or("CEX_PERSIST_GROUP", d.group),
            consumer: env_or("CEX_PERSIST_CONSUMER", d.consumer),
            count: env_num("CEX_PERSIST_COUNT", d.count),
            block_ms: env_num("CEX_BLOCK_MS", d.block_ms),
        }
    }
}

fn env_or(key: &str, fallback: String) -> String {
    std::env::var(key).unwrap_or(fallback)
}

fn env_num(key: &str, fallback: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}
