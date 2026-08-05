//! The persister process: `cex:events` in, Postgres out.
//!
//! A separate process because **the engine must never wait on a database**. The
//! engine publishes an event batch and moves on in microseconds; this service
//! catches up at whatever speed Postgres allows. Falling behind costs history
//! freshness and nothing else — the engine does not notice, and the durable
//! command log means nothing is lost.
//!
//! ## Why `XREADGROUP` and not plain `XREAD`
//!
//! The engine reads with plain `XREAD` because its snapshot already records
//! where it is; a second cursor would compete for that authority. This service
//! has no snapshot, so it wants exactly what a consumer group provides: Redis
//! tracks the cursor, and anything not acknowledged comes back.
//!
//! That makes delivery at-least-once, which is why every write is deduplicated
//! on `seq`.

pub mod config;
pub mod consumer;
pub mod store;

pub use config::Config;
pub use consumer::Consumer;
pub use store::{BalanceChangeRow, FillRow, HistoryStore, OrderRow, StoreError};
