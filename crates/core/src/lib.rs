//! The matching engine core.
//!
//! This crate is a pure function of `(state, command)`. It has no async runtime,
//! no network, no clock, and no randomness — every source of non-determinism
//! enters as a command from outside. That is what makes snapshot-and-replay
//! recovery honest rather than aspirational, and it is why the dependency list in
//! `Cargo.toml` is as short as it is.
//!
//! Floats are not used here and must not be introduced. See [`math`].

pub mod book;
pub mod error;
pub mod market;
pub mod math;

pub use book::{Order, OrderBook};
pub use error::EngineError;
pub use market::{Market, MarketRegistry};
pub use math::{mul_div, Rounding};
