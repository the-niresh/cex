//! The engine process: everything around `cex_core` that touches the outside
//! world.
//!
//! The split is deliberate. `cex-core` is a pure function of `(state, command)`
//! and has no I/O at all; this crate is where Redis, the filesystem, and the
//! clock live. Keeping them apart is what makes replay honest — the core cannot
//! read a clock even by accident, because the dependency is not in its manifest.

pub mod config;
pub mod runner;
pub mod snapshot_store;
pub mod stream_id;

pub use config::Config;
pub use runner::Runner;
pub use snapshot_store::SnapshotStore;
pub use stream_id::StreamId;
