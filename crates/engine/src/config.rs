use std::path::PathBuf;

use cex_proto::{CHANNEL_RESPONSES, STREAM_COMMANDS, STREAM_EVENTS};

/// Everything the engine process needs from its environment.
///
/// Stream names are configurable so tests can run against a real Redis without
/// colliding with each other or with a running engine.
#[derive(Debug, Clone)]
pub struct Config {
    pub redis_url: String,
    pub commands_stream: String,
    pub events_stream: String,
    pub responses_channel: String,
    pub snapshot_dir: PathBuf,
    /// Take a snapshot after this many applied commands.
    pub snapshot_every: usize,
    /// How many snapshots to retain on disk.
    pub snapshot_keep: usize,
    /// How long `XREAD` blocks waiting for new commands, in milliseconds.
    pub block_ms: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            // Non-default port: the compose file deliberately avoids clashing
            // with any other Redis on the box.
            redis_url: "redis://127.0.0.1:6390".into(),
            commands_stream: STREAM_COMMANDS.into(),
            events_stream: STREAM_EVENTS.into(),
            responses_channel: CHANNEL_RESPONSES.into(),
            snapshot_dir: PathBuf::from("data/snapshots"),
            snapshot_every: 5_000,
            snapshot_keep: 3,
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
            commands_stream: env_or("CEX_COMMANDS_STREAM", d.commands_stream),
            events_stream: env_or("CEX_EVENTS_STREAM", d.events_stream),
            responses_channel: env_or("CEX_RESPONSES_CHANNEL", d.responses_channel),
            snapshot_dir: env_or("CEX_SNAPSHOT_DIR", d.snapshot_dir.display().to_string()).into(),
            snapshot_every: env_num("CEX_SNAPSHOT_EVERY", d.snapshot_every),
            snapshot_keep: env_num("CEX_SNAPSHOT_KEEP", d.snapshot_keep),
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
