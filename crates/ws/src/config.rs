use cex_proto::STREAM_EVENTS;

/// Everything the market data process needs from its environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub redis_url: String,
    pub events_stream: String,
    /// Consumer group name. Separate from the persister's, so the two read the
    /// same stream without competing for entries.
    pub group: String,
    /// This instance's name within the group.
    ///
    /// Unlike the persister, a name that changes per boot is harmless here:
    /// this service acknowledges immediately and holds nothing durable, so an
    /// orphaned pending list would only ever contain updates that are already
    /// stale. Stability is still preferable — it keeps `XINFO GROUPS` readable.
    pub consumer: String,
    /// Address to listen on.
    pub bind: String,
    /// How many updates the broadcast ring holds before the slowest subscriber
    /// starts losing them — and, therefore, gets disconnected.
    pub broadcast_capacity: usize,
    pub count: usize,
    pub block_ms: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            redis_url: "redis://127.0.0.1:6390".into(),
            events_stream: STREAM_EVENTS.into(),
            group: "cex:ws".into(),
            consumer: "ws-1".into(),
            bind: "0.0.0.0:8081".into(),
            broadcast_capacity: 1024,
            count: 256,
            block_ms: 5_000,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let d = Config::default();
        Config {
            redis_url: env_or("CEX_REDIS_URL", d.redis_url),
            events_stream: env_or("CEX_EVENTS_STREAM", d.events_stream),
            group: env_or("CEX_WS_GROUP", d.group),
            consumer: env_or("CEX_WS_CONSUMER", d.consumer),
            bind: env_or("CEX_WS_BIND", d.bind),
            broadcast_capacity: env_num("CEX_WS_CAPACITY", d.broadcast_capacity),
            count: env_num("CEX_WS_COUNT", d.count),
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
